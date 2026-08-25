//! System clipboard get/set for text input fields (Ctrl+C / Ctrl+V).
//! Uses the arboard crate; may fail on headless or unsupported Wayland compositors.

use arboard::Clipboard;

#[cfg(target_os = "linux")]
use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};

/// Get text from the system clipboard. Returns None on failure or empty.
pub fn get() -> Option<String> {
    let mut clipboard = Clipboard::new().ok()?;
    #[cfg(target_os = "linux")]
    {
        for kind in [LinuxClipboardKind::Clipboard, LinuxClipboardKind::Primary] {
            if let Ok(text) = clipboard.get().clipboard(kind).text() {
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        return None;
    }
    #[cfg(not(target_os = "linux"))]
    {
        clipboard.get_text().ok().filter(|s: &String| !s.is_empty())
    }
}

/// Set the system clipboard to the given text. Ignores errors.
pub fn set(text: &str) {
    let Ok(mut clipboard) = Clipboard::new() else {
        return;
    };
    #[cfg(target_os = "linux")]
    {
        let _ = clipboard.set().wait().text(text.to_owned());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = clipboard.set_text(text.to_owned());
    }
}
