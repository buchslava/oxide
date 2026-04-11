//! Copy / move / delete / archive progress overlays.

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressPalette {
    pub background: Color,
    pub border: Color,
    pub gauge: Color,
    pub section_label: Color,
    pub path_text: Color,
    pub hint: Color,
}
