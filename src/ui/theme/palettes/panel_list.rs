//! File list (both panels).

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelListPalette {
    pub selected_fg: Color,
    pub selected_bg: Color,
    pub directory_fg: Color,
    pub executable_fg: Color,
    pub zip_fg: Color,
    pub symlink_fg: Color,
    pub file_fg: Color,
    /// Dotfiles (name starts with `.`, except `..`); only when row is not selected.
    pub hidden_fg: Color,
    /// `> ` and folder-diff `C ` / `S ` / `X ` in panel file lists.
    pub marked_prefix: Color,
}

impl PanelListPalette {
    /// Cursor row in the active panel: dark text on `selected_bg` (keeps the highlight fill).
    #[inline]
    pub fn selected_row_style(self) -> Style {
        Style::default()
            .fg(self.selected_fg)
            .bg(self.selected_bg)
            .add_modifier(Modifier::BOLD)
    }
}
