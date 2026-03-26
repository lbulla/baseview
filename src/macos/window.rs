use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApp, NSApplicationActivationPolicy, NSBackingStoreType, NSCursor, NSPasteboard,
    NSPasteboardTypeString, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
};

use crate::macos::cursor::cursor_to_nscursor;
use crate::macos::view::View;
use crate::{MouseCursor, Size, WindowHandler, WindowInfo, WindowOpenOptions, WindowScalePolicy};

#[cfg(feature = "opengl")]
use crate::gl::GlContext;

pub struct WindowHandle {
    /// Our subclassed NSView
    ns_view: Retained<View>,
}

impl WindowHandle {
    pub fn close(&mut self) {
        unsafe {
            self.ns_view.close();
        }
    }

    pub fn is_open(&self) -> bool {
        self.ns_view.is_open()
    }
}

impl HasWindowHandle for WindowHandle {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, HandleError> {
        if self.is_open() {
            let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(
                NonNull::from(self.ns_view.as_super()).cast(),
            ));
            unsafe { Ok(raw_window_handle::WindowHandle::borrow_raw(raw)) }
        } else {
            Err(HandleError::Unavailable)
        }
    }
}

pub struct Window<'a> {
    ns_view: &'a View,
}

impl<'a> Window<'a> {
    pub fn open_parented<P, H, B>(parent: P, options: WindowOpenOptions, build: B) -> WindowHandle
    where
        P: HasWindowHandle,
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        let mtm = MainThreadMarker::new().expect("Window must be opened on the main thread");

        let scaling = match options.scale {
            WindowScalePolicy::ScaleFactor(scale) => scale,
            WindowScalePolicy::SystemScaleFactor => 1.0,
        };

        let window_info = WindowInfo::from_logical_size(options.size, scaling);
        let ns_view = unsafe { View::new(options, window_info, None, None, mtm) };

        let parent_ns_view = if let RawWindowHandle::AppKit(handle) =
            parent.window_handle().expect("No window handle").as_raw()
        {
            handle.ns_view.as_ptr() as *mut NSView
        } else {
            panic!("Not a macOS window");
        };
        unsafe {
            (*parent_ns_view).addSubview(&ns_view);
        }

        let mut window = crate::Window::new(Window::new(&ns_view));
        let window_handler = Box::new(build(&mut window));
        ns_view.set_window_handler(window_handler);

        unsafe {
            View::setup_timer(&ns_view);
        }

        WindowHandle { ns_view }
    }

    pub fn open_blocking<H, B>(options: WindowOpenOptions, build: B)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        // It seems prudent to run NSApp() here before doing other
        // work. It runs [NSApplication sharedApplication], which is
        // what is run at the very start of the Xcode-generated main
        // function of a cocoa app according to:
        // https://developer.apple.com/documentation/appkit/nsapplication
        let mtm = MainThreadMarker::new().expect("Window must be opened on the main thread");
        let ns_app = NSApp(mtm);
        ns_app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

        let scaling = match options.scale {
            WindowScalePolicy::ScaleFactor(scale) => scale,
            WindowScalePolicy::SystemScaleFactor => 1.0,
        };

        let window_info = WindowInfo::from_logical_size(options.size, scaling);

        let rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(window_info.logical_size().width, window_info.logical_size().height),
        );

        let ns_window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        ns_window.center();

        let title = NSString::from_str(&options.title);
        ns_window.setTitle(&title);
        ns_window.makeKeyAndOrderFront(None);

        let ns_view = unsafe {
            View::new(options, window_info, Some(ns_app.clone()), Some(ns_window.clone()), mtm)
        };
        ns_window.setContentView(Some(&ns_view));
        ns_window.setDelegate(Some(ProtocolObject::from_ref(&*ns_view)));

        let mut window = crate::Window::new(Window::new(&ns_view));
        let window_handler = Box::new(build(&mut window));
        ns_view.set_window_handler(window_handler);

        unsafe {
            View::setup_timer(&ns_view);
        }

        let handle = WindowHandle { ns_view };

        ns_app.run();

        drop(handle);
    }

    pub fn close(&mut self) {
        unsafe {
            self.ns_view.close();
        }
    }

    pub fn has_focus(&self) -> bool {
        self.ns_view.has_focus()
    }

    pub fn focus(&mut self) {
        self.ns_view.focus();
    }

    pub fn resize(&mut self, size: Size) {
        unsafe {
            self.ns_view.resize(size);
        }
    }

    pub fn set_mouse_cursor(&mut self, mouse_cursor: MouseCursor) {
        unsafe {
            let ns_cursor = cursor_to_nscursor(mouse_cursor);
            ns_cursor.set();

            match mouse_cursor {
                MouseCursor::Hidden => NSCursor::hide(),
                _ => NSCursor::unhide(),
            }
        }
    }

    #[cfg(feature = "opengl")]
    pub fn gl_context(&self) -> Option<&GlContext> {
        self.ns_view.gl_context()
    }

    pub(crate) fn new(ns_view: &'a View) -> Self {
        Self { ns_view }
    }
}

impl<'a> HasWindowHandle for Window<'a> {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, HandleError> {
        if self.ns_view.is_open() {
            let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(
                NonNull::from(self.ns_view.as_super()).cast(),
            ));
            unsafe { Ok(raw_window_handle::WindowHandle::borrow_raw(raw)) }
        } else {
            Err(HandleError::Unavailable)
        }
    }
}

impl<'a> HasDisplayHandle for Window<'a> {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, HandleError> {
        Ok(raw_window_handle::DisplayHandle::appkit())
    }
}

pub fn copy_to_clipboard(string: &str) {
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        pb.setString_forType(&NSString::from_str(string), NSPasteboardTypeString);
    }
}
