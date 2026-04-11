//! Ctrl+D two-file diff (aligned lines, synchronized scroll).

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffViewerPalette {
    pub background: Color,
    pub text: Color,
    pub header_path: Color,
    pub muted: Color,
    pub column_border: Color,
    /// Row on the opposite pane when the other side has no line (typically same as [`Self::background`]).
    pub gap_bg: Color,
    /// Text color for gap cells (usually matches `gap_bg` when padding is spaces).
    pub gap_fg: Color,
    pub removed_bg: Color,
    pub removed_fg: Color,
    pub added_bg: Color,
    pub added_fg: Color,
    pub changed_old_bg: Color,
    pub changed_old_fg: Color,
    pub changed_new_bg: Color,
    pub changed_new_fg: Color,
    pub line_number_fg: Color,
    /// Intra-line removed chars (left pane, changed rows).
    pub char_removed_bg: Color,
    pub char_removed_fg: Color,
    /// Intra-line inserted chars (right pane, changed rows).
    pub char_added_bg: Color,
    pub char_added_fg: Color,
}
