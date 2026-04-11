//! Tiny helpers for theme tables (shorter than `Color::Rgb` at every call site).

use ratatui::style::Color;

#[inline]
pub const fn rgb(
    r: u8,
    g: u8,
    b: u8,
) -> Color {
    Color::Rgb(r, g, b)
}
