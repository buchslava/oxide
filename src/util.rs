//! Terminal layout helpers (depends on crossterm). Pure string/formatting lives in `core::text_format`.

use crossterm::terminal::size;

/// Compute visible panel height (rows) for layout and scroll. Terminal height minus
/// command line, menu bar, frame borders, and bottom bar.
pub(crate) fn compute_panel_height() -> usize {
    size()
        .map(|(_, h)| (h as usize).saturating_sub(5).max(1))
        .unwrap_or(18)
}
