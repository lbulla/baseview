use arboard::Clipboard;
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
))]
use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};

pub use arboard::Error;

pub fn copy_to_clipboard(data: &str, #[allow(unused)] primary: bool) -> Result<(), Error> {
    Clipboard::new().and_then(|mut clipboard| {
        let set = clipboard.set();

        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
        ))]
        let set = set.clipboard(if primary {
            LinuxClipboardKind::Primary
        } else {
            LinuxClipboardKind::Clipboard
        });

        set.text(data)
    })
}

pub fn paste_from_clipboard(#[allow(unused)] primary: bool) -> Result<String, Error> {
    Clipboard::new().and_then(|mut clipboard| {
        let get = clipboard.get();

        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
        ))]
        let get = get.clipboard(if primary {
            LinuxClipboardKind::Primary
        } else {
            LinuxClipboardKind::Clipboard
        });

        get.text()
    })
}
