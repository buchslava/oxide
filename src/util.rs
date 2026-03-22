//! Terminal layout and small I/O helpers (crossterm). Pure string/formatting lives in `core::text_format`.

use crossterm::terminal::size;
use std::io::{self, Write};

/// Log to stderr if the result is an error; otherwise ignore. Use for non-fatal I/O (e.g. save settings, refresh).
pub(crate) fn log_if_err(
    context: &str,
    res: io::Result<()>,
) {
    if let Err(e) = res {
        eprintln!("{}: {}", context, e);
    }
}

/// Defensive terminal reset after shell relay/commands (G0/G1, SGR, wrap).
pub(crate) fn reset_terminal_character_set_and_modes<W: Write>(out: &mut W) -> io::Result<()> {
    out.write_all(b"\x1b(B\x1b)B\x1b[0m\x1b[?7h")?;
    out.flush()?;
    Ok(())
}

/// Compute visible panel height (rows) for layout and scroll. Terminal height minus
/// command line, menu bar, frame borders, and bottom bar.
pub(crate) fn compute_panel_height() -> usize {
    size()
        .map(|(_, h)| (h as usize).saturating_sub(5).max(1))
        .unwrap_or(18)
}
