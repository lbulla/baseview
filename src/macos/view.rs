use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;

use core_foundation::runloop::{
    __CFRunLoopTimer, kCFRunLoopCommonModes, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext,
};
use keyboard_types::KeyboardEvent;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, MessageReceiver, ProtocolObject};
use objc2::{
    define_class, msg_send, sel, AnyThread, ClassType, DefinedClass, MainThreadMarker,
    MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingInfo, NSEvent, NSPasteboardTypeFileURL,
    NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowDelegate,
};
use objc2_foundation::{
    NSArray, NSNotification, NSNotificationCenter, NSNotificationName, NSObjectProtocol, NSPoint,
    NSRect, NSSize, NSString,
};

#[cfg(feature = "opengl")]
use crate::gl::GlContext;
use crate::macos::keyboard::KeyboardState;
use crate::macos::Window;
use crate::MouseEvent::{ButtonPressed, ButtonReleased};
use crate::{
    DropData, DropEffect, Event, EventStatus, MouseButton, MouseEvent, Point, ScrollDelta, Size,
    WindowEvent, WindowHandler, WindowInfo, WindowOpenOptions,
};

use super::keyboard::make_modifiers;

pub(crate) struct Ivars {
    open: Cell<bool>,
    window_handler: RefCell<Option<Box<dyn WindowHandler>>>,
    keyboard_state: KeyboardState,
    frame_timer: Cell<Option<CFRunLoopTimer>>,
    /// The last known window info for this window.
    window_info: Cell<WindowInfo>,
    /// Events that will be triggered at the end of `window_handler`'s borrow.
    deferred_events: RefCell<VecDeque<Event>>,
    /// If a new frame is requested while processing events, delay it until processing is done.
    pending_frame: Cell<bool>,

    /// Only set if we created the parent window, i.e. we are running in
    /// parentless mode
    ns_app: RefCell<Option<Retained<NSApplication>>>,
    /// Only set if we created the parent window, i.e. we are running in
    /// parentless mode
    ns_window: RefCell<Option<Retained<NSWindow>>>,

    #[cfg(feature = "opengl")]
    gl_context: Option<GlContext>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    pub(crate) struct View;

    impl View {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> Bool {
            Bool::YES
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> Bool {
            let is_key_window =
                if let Some(ns_window) = self.window() { ns_window.isKeyWindow() } else { false };
            if is_key_window {
                self.trigger_deferrable_event(Event::Window(WindowEvent::Focused));
            }
            Bool::YES
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> Bool {
            let is_key_window =
                if let Some(ns_window) = self.window() { ns_window.isKeyWindow() } else { false };
            if is_key_window {
                self.trigger_deferrable_event(Event::Window(WindowEvent::Unfocused));
            }
            Bool::YES
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> Bool {
            Bool::YES
        }

        #[unsafe(method(preservesContentInLiveResize))]
        fn preserves_content_in_live_resize(&self) -> Bool {
            Bool::NO
        }

        #[unsafe(method(acceptsFirstMouse))]
        fn accepts_first_mouse(&self) -> Bool {
            Bool::YES
        }


        #[unsafe(method(viewWillMoveToWindow:))]
        unsafe fn view_will_move_to_window(&self, new_window: Option<&NSWindow>) {
            let tracking_areas = self.trackingAreas();
            let tracking_area_count = tracking_areas.count();

            if let Some(new_window) = new_window {
                if tracking_area_count == 0 {
                    self.addTrackingArea(&self.create_tracking_area());
                }

                new_window.acceptsMouseMovedEvents();
                new_window.makeFirstResponder(Some(self));
            } else {
                if tracking_area_count != 0 {
                    let tracking_area = tracking_areas.objectAtIndex(0);
                    self.removeTrackingArea(&tracking_area);
                }
            }

            let superclass = self.class().superclass().unwrap();
            unsafe {
                let () = msg_send![super(self, superclass), viewWillMoveToWindow: new_window];
            }
        }

        #[unsafe(method(updateTrackingAreas:))]
        unsafe fn update_tracking_areas(&self, _: &AnyObject) {
            let tracking_areas = self.trackingAreas();
            let tracking_area = tracking_areas.objectAtIndex(0);
            self.removeTrackingArea(&tracking_area);
            self.addTrackingArea(&self.create_tracking_area());
        }

        #[unsafe(method(mouseMoved:))]
        unsafe fn mouse_moved(&self, event: &NSEvent) {
            self.trigger_mouse_move(event);
        }

        #[unsafe(method(mouseDragged:))]
        unsafe fn mouse_dragged(&self, event: &NSEvent) {
            self.trigger_mouse_move(event);
        }

        #[unsafe(method(rightMouseDragged:))]
        unsafe fn right_mouse_dragged(&self, event: &NSEvent) {
            self.trigger_mouse_move(event);
        }

        #[unsafe(method(otherMouseDragged:))]
        unsafe fn other_mouse_dragged(&self, event: &NSEvent) {
            self.trigger_mouse_move(event);
        }

        #[unsafe(method(scrollWheel:))]
        unsafe fn scroll_wheel(&self, event: &NSEvent) {
            let delta = {
                let x = event.scrollingDeltaX() as f32;
                let y = event.scrollingDeltaY() as f32;

                if event.hasPreciseScrollingDeltas() {
                    ScrollDelta::Pixels { x, y }
                } else {
                    ScrollDelta::Lines { x, y }
                }
            };

            let modifiers = NSEvent::modifierFlags(event);

            self.trigger_event(Event::Mouse(MouseEvent::WheelScrolled {
                delta,
                modifiers: make_modifiers(modifiers),
            }));
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            let scale_factor = self.scale_factor();
            let bounds = self.bounds();
            let new_window_info = WindowInfo::from_logical_size(
                Size::new(bounds.size.width, bounds.size.height),
                scale_factor,
            );

            let window_info = self.ivars().window_info.get();
            // Only send the event when the window's size has actually changed to be in line with the
            // other platform implementations
            if new_window_info.physical_size() == window_info.physical_size() {
                return;
            }

            self.ivars().window_info.set(new_window_info);
            self.trigger_deferrable_event(Event::Window(WindowEvent::Resized(new_window_info)));
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let scale_factor = self.scale_factor();
            let new_window_info = WindowInfo::from_logical_size(
                Size::new(size.width, size.height),
                scale_factor,
            );
            self.ivars().window_info.set(new_window_info);
            self.trigger_deferrable_event(Event::Window(WindowEvent::Resized(new_window_info)));

            unsafe {
                let () = self.send_super_message(NSView::class(), sel!(setFrameSize:), (size,));
            }
        }

        #[unsafe(method(draggingEntered:))]
        unsafe fn dragging_entered(
            &self, sender: &ProtocolObject<dyn NSDraggingInfo>
        ) -> NSDragOperation {
            let modifiers = self.keyboard_state().last_mods();
            let drop_data = Self::get_drop_data(sender);

            let event = MouseEvent::DragEntered {
                position: Self::get_drag_position(sender),
                modifiers: make_modifiers(modifiers),
                data: drop_data,
            };

            self.on_event(event)
        }

        #[unsafe(method(draggingUpdated:))]
        unsafe fn dragging_updated(
            &self, sender: &ProtocolObject<dyn NSDraggingInfo>,
        ) -> NSDragOperation {
            let modifiers = self.keyboard_state().last_mods();
            let drop_data = Self::get_drop_data(sender);

            let event = MouseEvent::DragMoved {
                position: Self::get_drag_position(sender),
                modifiers: make_modifiers(modifiers),
                data: drop_data,
            };

            self.on_event(event)
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag_operation(
            &self, _sender: &ProtocolObject<dyn NSDraggingInfo>
        ) -> Bool {
            // Always accept drag operation if we get this far
            // This function won't be called unless dragging_entered/updated
            // has returned an acceptable operation
            Bool::YES
        }

        #[unsafe(method(performDragOperation:))]
        unsafe fn perform_drag_operation(
            &self, sender: &ProtocolObject<dyn NSDraggingInfo>,
        ) -> Bool {
            let modifiers = self.keyboard_state().last_mods();
            let drop_data = Self::get_drop_data(sender);

            let event = MouseEvent::DragDropped {
                position: Self::get_drag_position(sender),
                modifiers: make_modifiers(modifiers),
                data: drop_data,
            };

            let event_status = self.trigger_event(Event::Mouse(event));
            match event_status {
                EventStatus::AcceptDrop(_) => Bool::YES,
                _ => Bool::NO,
            }
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(
            &self, sender: &ProtocolObject<dyn NSDraggingInfo>,
        ) {
            self.on_event(MouseEvent::DragLeft);
        }

        #[unsafe(method(handleNotification:))]
        fn handle_notification(&self, notification: &NSNotification) {
            let Some(ns_window) = self.window() else {
                return;
            };
            let Some(first_responder) = ns_window.firstResponder() else {
                return;
            };

            // Only trigger focus events if the NSWindow that's being notified about is our window,
            // and if the window's first responder is our NSView.
            // If the first responder isn't our NSView, the focus events will instead be triggered
            // by the becomeFirstResponder and resignFirstResponder methods on the NSView itself.
            if ns_window.isEqual(Some(notification)) && self.isEqual(Some(&first_responder)) {
                let is_key_window = ns_window.isKeyWindow();
                self.trigger_event(Event::Window(if is_key_window {
                    WindowEvent::Focused
                } else {
                    WindowEvent::Unfocused
                }));
            }
        }

        #[unsafe(method(mouseDown:))]
        unsafe fn mouse_down(&self, event: &NSEvent) {
            let modifiers = event.modifierFlags();

            self.trigger_event(Event::Mouse(ButtonPressed {
                button: MouseButton::Left,
                modifiers: make_modifiers(modifiers),
            }));
        }

        #[unsafe(method(mouseUp:))]
        unsafe fn mouse_up(&self, event: &NSEvent) {
            let modifiers = event.modifierFlags();

            self.trigger_event(Event::Mouse(ButtonReleased {
                button: MouseButton::Left,
                modifiers: make_modifiers(modifiers),
            }));
        }

        #[unsafe(method(rightMouseDown:))]
        unsafe fn right_mouse_down(&self, event: &NSEvent) {
            let modifiers = event.modifierFlags();

            self.trigger_event(Event::Mouse(ButtonPressed {
                button: MouseButton::Right,
                modifiers: make_modifiers(modifiers),
            }));
        }

        #[unsafe(method(rightMouseUp:))]
        unsafe fn right_mouse_up(&self, event: &NSEvent) {
            let modifiers = event.modifierFlags();

            self.trigger_event(Event::Mouse(ButtonReleased {
                button: MouseButton::Right,
                modifiers: make_modifiers(modifiers),
            }));
        }

        #[unsafe(method(otherMouseDown:))]
        unsafe fn other_mouse_down(&self, event: &NSEvent) {
            let modifiers = event.modifierFlags();

            self.trigger_event(Event::Mouse(ButtonPressed {
                button: MouseButton::Middle,
                modifiers: make_modifiers(modifiers),
            }));
        }

        #[unsafe(method(otherMouseUp:))]
        unsafe fn other_mouse_up(&self, event: &NSEvent) {
            let modifiers = event.modifierFlags();

            self.trigger_event(Event::Mouse(ButtonReleased {
                button: MouseButton::Middle,
                modifiers: make_modifiers(modifiers),
            }));
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.trigger_event(Event::Mouse(MouseEvent::CursorEntered));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            self.trigger_event(Event::Mouse(MouseEvent::CursorLeft));
        }

        #[unsafe(method(keyDown:))]
        unsafe fn key_down(&self, event: &NSEvent) {
            if let Some(key_event) = self.process_native_key_event(event){
                let status = self.trigger_event(Event::Keyboard(key_event));

                if let EventStatus::Ignored = status {
                    unsafe {
                        let () = self.send_super_message(NSView::class(), sel!(keyDown), (event,));
                    }
                }
            }
        }

        #[unsafe(method(keyUp:))]
        unsafe fn key_up(&self, event: &NSEvent) {
            if let Some(key_event) = self.process_native_key_event(event){
                let status = self.trigger_event(Event::Keyboard(key_event));

                if let EventStatus::Ignored = status {
                    unsafe {
                        let () = self.send_super_message(NSView::class(), sel!(keyUp), (event,));
                    }
                }
            }
        }

        #[unsafe(method(flagsChanged:))]
        unsafe fn flags_changed(&self, event: &NSEvent) {
            if let Some(key_event) = self.process_native_key_event(event){
                let status = self.trigger_event(Event::Keyboard(key_event));

                if let EventStatus::Ignored = status {
                    unsafe {
                        let () = self.send_super_message(NSView::class(), sel!(flagsChanged), (event,));
                    }
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for View {}

    unsafe impl NSWindowDelegate for View {
        #[unsafe(method(windowShouldClose:))]
        unsafe fn window_should_close(&self, _notification: &NSNotification) -> Bool {
            self.close();
            Bool::NO
        }
    }
);

impl View {
    pub(crate) unsafe fn new(
        options: WindowOpenOptions, window_info: WindowInfo,
        ns_app: Option<Retained<NSApplication>>, ns_window: Option<Retained<NSWindow>>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm);

        #[cfg(feature = "opengl")]
        let gl_context = options.gl_config.map(|gl_config| {
            use std::ptr::NonNull;

            use objc2::rc::Allocated;
            use raw_window_handle::{AppKitWindowHandle, RawWindowHandle};

            let parent = RawWindowHandle::AppKit(AppKitWindowHandle::new(
                NonNull::new(Allocated::as_ptr(&this) as _).unwrap(),
            ));
            GlContext::create(&parent, gl_config, mtm).expect("Could not create OpenGL context")
        });

        let frame =
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(options.size.width, options.size.height));
        let this = this.set_ivars(Ivars {
            open: Cell::new(true),
            window_handler: RefCell::new(None),
            keyboard_state: KeyboardState::new(),
            frame_timer: Cell::new(None),
            window_info: Cell::new(window_info),
            deferred_events: RefCell::default(),
            pending_frame: Cell::new(false),

            ns_app: RefCell::new(ns_app),
            ns_window: RefCell::new(ns_window),

            #[cfg(feature = "opengl")]
            gl_context,
        });
        let this: Retained<Self> = msg_send![super(this), initWithFrame: frame];

        let notification_center = NSNotificationCenter::defaultCenter();
        notification_center.addObserver_selector_name_object(
            &this,
            sel!(handleNotification:),
            Some(&NSNotificationName::from_selector(sel!(NSWindowDidBecomeKeyNotification))),
            None,
        );
        this.registerForDraggedTypes(&NSArray::arrayWithObject(NSPasteboardTypeFileURL));

        this
    }

    pub(crate) fn set_window_handler(&self, window_handler: Box<dyn WindowHandler>) {
        self.ivars().window_handler.borrow_mut().replace(window_handler);
    }

    pub(crate) fn set_timer(&self, timer: CFRunLoopTimer) {
        self.ivars().frame_timer.set(Some(timer));
    }

    pub(crate) unsafe fn setup_timer(ns_view: &Retained<View>) {
        extern "C" fn timer_callback(_: *mut __CFRunLoopTimer, ns_view: *mut c_void) {
            unsafe {
                let ns_view = &*(ns_view as *const View);
                ns_view.trigger_frame();
            }
        }

        let mut timer_context = CFRunLoopTimerContext {
            version: 0,
            info: Retained::as_ptr(ns_view) as _,
            retain: None,
            release: None,
            copyDescription: None,
        };

        let timer = CFRunLoopTimer::new(0.0, 0.015, 0, 0, timer_callback, &mut timer_context);
        CFRunLoop::get_current().add_timer(&timer, kCFRunLoopCommonModes);
        ns_view.set_timer(timer);
    }

    pub(crate) unsafe fn close(&self) {
        if !self.is_open() {
            return;
        }

        self.trigger_event(Event::Window(WindowEvent::WillClose));

        // Close the window if in non-parented mode
        if let Some(ns_window) = self.ivars().ns_window.take() {
            ns_window.close();
        }

        self.ivars().open.set(false);

        // If in non-parented mode, we want to also quit the app altogether
        if let Some(app) = self.ivars().ns_app.take() {
            app.stop(Some(&app));
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.ivars().open.get()
    }

    pub(crate) fn has_focus(&self) -> bool {
        let Some(ns_window) = self.window() else {
            return false;
        };

        let first_responder = ns_window.firstResponder();
        let is_key_window = ns_window.isKeyWindow();
        let is_focused = self.isEqual(first_responder.as_ref().map(|r| r.as_ref()));
        is_key_window && is_focused
    }

    pub(crate) fn focus(&self) {
        let Some(ns_window) = self.window() else {
            return;
        };

        ns_window.makeFirstResponder(Some(self));
    }

    pub(crate) unsafe fn resize(&self, size: Size) {
        if self.is_open() {
            // NOTE: macOS gives you a personal rave if you pass in fractional pixels here. Even
            // though the size is in fractional pixels.
            let ns_size = NSSize::new(size.width.round(), size.height.round());

            self.setFrameSize(ns_size);
            self.setNeedsDisplay(true);

            // When using OpenGL the `NSOpenGLView` needs to be resized separately? Why? Because
            // macOS.
            #[cfg(feature = "opengl")]
            if let Some(gl_context) = self.ivars().gl_context.as_ref() {
                gl_context.resize(ns_size);
            }

            // If this is a standalone window then we'll also need to resize the window itself
            if let Some(ns_window) = self.ivars().ns_window.borrow().as_ref() {
                ns_window.setContentSize(ns_size);
            }
        }
    }

    #[cfg(feature = "opengl")]
    pub(crate) fn gl_context(&self) -> Option<&GlContext> {
        self.ivars().gl_context.as_ref()
    }

    unsafe fn create_tracking_area(&self) -> Retained<NSTrackingArea> {
        let bounds = self.bounds();
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::CursorUpdate
            | NSTrackingAreaOptions::ActiveInActiveApp
            | NSTrackingAreaOptions::InVisibleRect
            | NSTrackingAreaOptions::EnabledDuringMouseDrag;

        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            bounds,
            options,
            Some(self),
            None,
        )
    }

    unsafe fn get_drag_position(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Point {
        let point = sender.draggingLocation();
        Point::new(point.x, point.y)
    }

    unsafe fn get_drop_data(sender: &ProtocolObject<dyn NSDraggingInfo>) -> DropData {
        let pasteboard = sender.draggingPasteboard();
        let Some(file_list) = pasteboard.propertyListForType(NSPasteboardTypeFileURL) else {
            return DropData::None;
        };

        let file_list = Retained::cast_unchecked::<NSArray<NSString>>(file_list);
        let mut files = vec![];
        for i in 0..file_list.count() {
            let data = file_list.objectAtIndex(i);
            files.push(data.to_string().into());
        }

        DropData::Files(files)
    }

    fn keyboard_state(&self) -> &KeyboardState {
        &self.ivars().keyboard_state
    }

    fn on_event(&self, event: MouseEvent) -> NSDragOperation {
        let event_status = self.trigger_event(Event::Mouse(event));
        match event_status {
            EventStatus::AcceptDrop(DropEffect::Copy) => NSDragOperation::Copy,
            EventStatus::AcceptDrop(DropEffect::Move) => NSDragOperation::Move,
            EventStatus::AcceptDrop(DropEffect::Link) => NSDragOperation::Link,
            EventStatus::AcceptDrop(DropEffect::Scroll) => NSDragOperation::Generic,
            _ => NSDragOperation::None,
        }
    }

    unsafe fn process_native_key_event(&self, event: &NSEvent) -> Option<KeyboardEvent> {
        self.keyboard_state().process_native_event(event)
    }

    fn scale_factor(&self) -> f64 {
        if let Some(ns_window) = self.window() {
            ns_window.backingScaleFactor()
        } else {
            self.ivars().window_info.get().scale()
        }
    }

    fn send_deferred_events(&self, window_handler: &mut dyn WindowHandler) {
        let mut window = crate::Window::new(Window::new(self));
        loop {
            let next_event = self.ivars().deferred_events.borrow_mut().pop_front();
            if let Some(event) = next_event {
                window_handler.on_event(&mut window, event);
            } else {
                break;
            }
        }
    }

    /// Trigger the event immediately if `window_handler` can be borrowed mutably,
    /// otherwise add the event to a queue that will be cleared once `window_handler`'s mutable borrow ends.
    /// As this method might result in the event triggering asynchronously, it can't reliably return the event status.
    fn trigger_deferrable_event(&self, event: Event) {
        if let Ok(mut window_handler) = self.ivars().window_handler.try_borrow_mut() {
            if let Some(window_handler) = window_handler.as_mut() {
                let mut window = crate::Window::new(Window::new(self));
                window_handler.on_event(&mut window, event);
                return;
            }
        }
        self.ivars().deferred_events.borrow_mut().push_back(event);
    }

    /// Trigger the event immediately and return the event status.
    /// Will panic if `window_handler` is already borrowed or None (see `trigger_deferrable_event`).
    fn trigger_event(&self, event: Event) -> EventStatus {
        let mut window_handler = self.ivars().window_handler.borrow_mut();
        let window_handler = window_handler.as_mut().unwrap();
        self.send_deferred_events(window_handler.as_mut());
        let mut window = crate::Window::new(Window::new(self));
        let status = window_handler.on_event(&mut window, event);

        if self.ivars().pending_frame.get() {
            window_handler.on_frame(&mut window);
            self.ivars().pending_frame.set(false);
        }

        status
    }

    unsafe fn trigger_mouse_move(&self, event: &NSEvent) {
        let point: NSPoint = {
            let point = event.locationInWindow();
            self.convertPoint_fromView(point, None)
        };
        let modifiers = event.modifierFlags();
        let position = Point { x: point.x, y: point.y };

        self.trigger_event(Event::Mouse(MouseEvent::CursorMoved {
            position,
            modifiers: make_modifiers(modifiers),
        }));
    }

    fn trigger_frame(&self) {
        if let Ok(mut window_handler) = self.ivars().window_handler.try_borrow_mut() {
            let window_handler = window_handler.as_mut().unwrap();
            let mut window = crate::Window::new(Window::new(self));
            window_handler.on_frame(&mut window);
            self.send_deferred_events(window_handler.as_mut());
            self.ivars().pending_frame.set(false);
        } else {
            self.ivars().pending_frame.set(true);
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            if let Some(frame_timer) = self.ivars().frame_timer.take() {
                CFRunLoop::get_current().remove_timer(&frame_timer, kCFRunLoopCommonModes);
            }

            // Deregister NSView from NotificationCenter.
            let notification_center = NSNotificationCenter::defaultCenter();
            notification_center.removeObserver(self);

            // Ensure that the NSView is detached from the parent window
            self.removeFromSuperview();
        }
    }
}
