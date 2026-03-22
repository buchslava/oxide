//! System clipboard get/set for text input fields (Ctrl+C / Ctrl+V).
//! Uses the arboard crate; may fail on headless or some Wayland setups.

/// Get text from the system clipboard. Returns None on failure or empty.
pub fn get() -> Option<String> {
    arboard::Clipboard::new()
        .ok()?
        .get_text()
        .ok()
        .filter(|s: &String| !s.is_empty())
}

/// Set the system clipboard to the given text. Ignores errors.
pub fn set(text: &str) {
    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string()));
}
