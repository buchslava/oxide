//! F3 viewer.

use ratatui::style::{Color, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerPalette {
    pub background: Color,
    pub text: Color,
    pub header_path: Color,
    pub muted: Color,
    pub hex_cursor_bg: Color,
    pub hex_cursor_fg: Color,
}

impl ViewerPalette {
    #[inline]
    pub fn hex_cursor_highlight_style(self) -> Style {
        Style::default()
            .bg(self.hex_cursor_bg)
            .fg(self.hex_cursor_fg)
    }
}
