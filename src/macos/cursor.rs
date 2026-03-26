use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{msg_send, sel, AnyThread, ClassType};
use objc2_app_kit::{
    NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition, NSImage,
};
use objc2_foundation::{ns_string, NSDictionary, NSNumber, NSObject, NSPoint, NSString};

use crate::MouseCursor;

pub(crate) unsafe fn cursor_to_nscursor(mouse_cursor: MouseCursor) -> Retained<NSCursor> {
    match mouse_cursor {
        MouseCursor::Crosshair => NSCursor::crosshairCursor(),
        MouseCursor::Default => default_cursor(),
        MouseCursor::Ptr => NSCursor::pointingHandCursor(),
        MouseCursor::Hand => NSCursor::openHandCursor(),
        MouseCursor::HandGrabbing => NSCursor::closedHandCursor(),
        MouseCursor::Help => _helpCursor(),
        MouseCursor::Hidden => default_cursor(),
        MouseCursor::Text => NSCursor::IBeamCursor(),
        MouseCursor::VerticalText => NSCursor::IBeamCursorForVerticalLayout(),
        MouseCursor::Working => busyButClickableCursor(),
        MouseCursor::PtrWorking => busyButClickableCursor(),
        MouseCursor::NotAllowed => NSCursor::operationNotAllowedCursor(),
        MouseCursor::PtrNotAllowed => NSCursor::operationNotAllowedCursor(),
        MouseCursor::ZoomIn => NSCursor::zoomInCursor(),
        MouseCursor::ZoomOut => NSCursor::zoomOutCursor(),
        MouseCursor::Alias => NSCursor::dragLinkCursor(),
        MouseCursor::Copy => NSCursor::dragCopyCursor(),
        MouseCursor::Move => webkit_move(),
        MouseCursor::AllScroll => webkit_move(),
        MouseCursor::Cell => webkit_cell(),
        MouseCursor::EResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::Right,
            NSCursorFrameResizeDirections::Outward,
        ),
        MouseCursor::NResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::Top,
            NSCursorFrameResizeDirections::Outward,
        ),
        MouseCursor::NeResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::TopRight,
            NSCursorFrameResizeDirections::Outward,
        ),
        MouseCursor::NwResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::TopLeft,
            NSCursorFrameResizeDirections::Outward,
        ),
        MouseCursor::SResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::Bottom,
            NSCursorFrameResizeDirections::Outward,
        ),
        MouseCursor::SeResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::BottomRight,
            NSCursorFrameResizeDirections::Outward,
        ),
        MouseCursor::SwResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::BottomLeft,
            NSCursorFrameResizeDirections::Outward,
        ),
        MouseCursor::WResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::Left,
            NSCursorFrameResizeDirections::Outward,
        ),
        MouseCursor::EwResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::Right,
            NSCursorFrameResizeDirections::All,
        ),
        MouseCursor::NsResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::Top,
            NSCursorFrameResizeDirections::All,
        ),
        MouseCursor::NwseResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::TopLeft,
            NSCursorFrameResizeDirections::All,
        ),
        MouseCursor::NeswResize => NSCursor::frameResizeCursorFromPosition_inDirections(
            NSCursorFrameResizePosition::TopRight,
            NSCursorFrameResizeDirections::All,
        ),
        MouseCursor::ColResize => NSCursor::columnResizeCursor(),
        MouseCursor::RowResize => NSCursor::rowResizeCursor(),
    }
}

// Taken from https://github.com/rust-windowing/winit/blob/master/winit-appkit/src/cursor.rs.

fn default_cursor() -> Retained<NSCursor> {
    NSCursor::arrowCursor()
}

unsafe fn try_cursor_from_selector(sel: Sel) -> Option<Retained<NSCursor>> {
    let cls = NSCursor::class();
    if unsafe { msg_send![cls, respondsToSelector: sel] } {
        let cursor: Retained<NSCursor> = unsafe { msg_send![cls, performSelector: sel] };
        Some(cursor)
    } else {
        println!("cursor `{sel}` appears to be invalid");
        None
    }
}

macro_rules! def_undocumented_cursor {
    {$(
        $(#[$($m:meta)*])*
        fn $name:ident();
    )*} => {$(
        $(#[$($m)*])*
        #[allow(non_snake_case)]
        fn $name() -> Retained<NSCursor> {
            unsafe { try_cursor_from_selector(sel!($name)).unwrap_or_else(|| default_cursor()) }
        }
    )*};
}

def_undocumented_cursor!(
    // Undocumented cursors: https://stackoverflow.com/a/46635398/5435443
    fn _helpCursor();
    fn _zoomInCursor();
    fn _zoomOutCursor();
    fn _windowResizeNorthEastCursor();
    fn _windowResizeNorthWestCursor();
    fn _windowResizeSouthEastCursor();
    fn _windowResizeSouthWestCursor();
    fn _windowResizeNorthEastSouthWestCursor();
    fn _windowResizeNorthWestSouthEastCursor();

    // While these two are available, the former just loads a white arrow,
    // and the latter loads an ugly deflated beachball!
    // pub fn _moveCursor();
    // pub fn _waitCursor();

    // An even more undocumented cursor...
    // https://bugs.eclipse.org/bugs/show_bug.cgi?id=522349
    fn busyButClickableCursor();
);

unsafe fn load_webkit_cursor(name: &NSString) -> Retained<NSCursor> {
    // Snatch a cursor from WebKit; They fit the style of the native
    // cursors, and will seem completely standard to macOS users.
    //
    // https://stackoverflow.com/a/21786835/5435443
    let root = ns_string!(
        "/System/Library/Frameworks/ApplicationServices.framework/Versions/A/Frameworks/\
         HIServices.framework/Versions/A/Resources/cursors"
    );
    let cursor_path = root.stringByAppendingPathComponent(name);

    let pdf_path = cursor_path.stringByAppendingPathComponent(ns_string!("cursor.pdf"));
    let image = NSImage::initByReferencingFile(NSImage::alloc(), &pdf_path).unwrap();

    // TODO: Handle PLists better
    let info_path = cursor_path.stringByAppendingPathComponent(ns_string!("info.plist"));
    #[allow(deprecated)]
    let info: Retained<NSDictionary<NSObject, NSObject>> =
        unsafe { NSDictionary::dictionaryWithContentsOfFile(&info_path) }.unwrap();
    let mut x = 0.0;
    if let Some(n) = info.objectForKey(ns_string!("hotx")) {
        if let Ok(n) = n.downcast::<NSNumber>() {
            x = n.as_cgfloat();
        }
    }
    let mut y = 0.0;
    if let Some(n) = info.objectForKey(ns_string!("hoty")) {
        if let Ok(n) = n.downcast::<NSNumber>() {
            y = n.as_cgfloat();
        }
    }

    let hotspot = NSPoint::new(x, y);
    NSCursor::initWithImage_hotSpot(NSCursor::alloc(), &image, hotspot)
}

fn webkit_move() -> Retained<NSCursor> {
    unsafe { load_webkit_cursor(ns_string!("move")) }
}

fn webkit_cell() -> Retained<NSCursor> {
    unsafe { load_webkit_cursor(ns_string!("cell")) }
}
