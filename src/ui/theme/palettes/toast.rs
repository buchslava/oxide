//! Toasts (ratatui widget + crossterm main-buffer overlay).

use crossterm::style::Color as CrosstermColor;
use ratatui::style::{Color, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastPalette {
    pub background: Color,
    pub foreground: Color,
    /// Warnings and errors (e.g. editor limit, settings save failure).
    pub alert_background: Color,
    pub alert_foreground: Color,
}

impl ToastPalette {
    #[inline]
    pub fn ratatui_style(self) -> Style {
        Style::default().bg(self.background).fg(self.foreground)
    }

    #[inline]
    pub fn ratatui_style_alert(self) -> Style {
        Style::default()
            .bg(self.alert_background)
            .fg(self.alert_foreground)
    }

    #[inline]
    pub fn crossterm_bg(self) -> CrosstermColor {
        rgb_to_crossterm(self.background)
    }

    #[inline]
    pub fn crossterm_fg(self) -> CrosstermColor {
        rgb_to_crossterm(self.foreground)
    }
}

fn rgb_to_crossterm(c: Color) -> CrosstermColor {
    match c {
        Color::Rgb(r, g, b) => CrosstermColor::Rgb { r, g, b },
        Color::Reset => CrosstermColor::Reset,
        _ => CrosstermColor::Reset,
    }
}
