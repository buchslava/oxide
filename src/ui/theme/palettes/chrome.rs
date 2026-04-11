//! Main panels, frame, command line, bottom bar, F10 menu.

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromePalette {
    pub main_background: Color,
    pub panel_border_fg: Color,
    pub panel_border_bg: Color,
    /// Typed command text on the bottom line.
    pub command_line_fg: Color,
    /// Directory + sigil prefix when the command line has focus.
    pub command_prompt_active_fg: Color,
    /// Same prefix when focus is on the panels (or elsewhere).
    pub command_prompt_inactive_fg: Color,
    pub menu_overlay_bg: Color,
    pub menu_hotkey: Color,
    pub menu_label: Color,
    pub menu_unavailable: Color,
    pub bottom_bar_path: Color,
    pub bottom_bar_size: Color,
    pub bottom_bar_success: Color,
    pub column_separator: Color,
}
