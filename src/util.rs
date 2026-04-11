//! Terminal layout, small I/O helpers (crossterm), and cooperative file reads for viewer/editor loads.
//! Pure string/formatting lives in `core::text_format`.

use crossterm::terminal::size;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

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

const READ_CHUNK: usize = 1024 * 1024;

/// Read a file in chunks, checking `cancel` between chunks (viewer/editor background loads).
pub(crate) fn read_path_chunked(
    path: &Path,
    cancel: &AtomicBool,
) -> io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let len_usize = usize::try_from(len).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "file size does not fit in usize",
        )
    })?;
    let mut buf = Vec::new();
    buf.try_reserve_exact(len_usize)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let mut scratch = vec![0u8; READ_CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "load cancelled",
            ));
        }
        let n = f.read(&mut scratch)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&scratch[..n]);
    }
    Ok(buf)
}
