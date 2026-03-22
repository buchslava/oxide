//! State for Ctrl+Q / Ctrl+W panel settings overlay. Shared by app_state and panel_overlay.

/// State for panel settings overlay. content_focus: 0 = View, 1 = Sort, 2 = Folders first, 3 = Show hidden.
#[derive(Debug, Clone)]
pub struct PanelSettingsOverlayState {
    pub content_focus: usize,
}

impl Default for PanelSettingsOverlayState {
    fn default() -> Self {
        Self { content_focus: 0 }
    }
}
