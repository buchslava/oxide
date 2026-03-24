//! Countdown overlay on the main terminal buffer (shell output still visible).

use crossterm::{
    cursor::MoveTo,
    execute,
    style::{Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use std::io::{self, Write};


fn countdown_secs_remaining(reveal_at: std::time::Instant) -> u64 {
    let now = std::time::Instant::now();
    if now >= reveal_at {
        return 0;
    }
    let rem = reveal_at - now;
    rem.as_secs() + u64::from(rem.subsec_nanos() > 0)
}

/// Bottom-left: `N sec` (+ trailing spaces to clear a longer previous line, e.g. `10 sec` → `9 sec`).
pub fn paint_main_buffer_countdown(app: &crate::app::state::AppState) -> io::Result<()> {
    let Some(cd) = app.post_command_countdown.as_ref() else {
        return Ok(());
    };
    if !cd.overlay_on_main_buffer {
        return Ok(());
    }
    let secs = countdown_secs_remaining(cd.reveal_at);
    let (cols, rows) = crossterm::terminal::size()?;
    let rows = rows.max(1);
    let y = rows.saturating_sub(1);
    let cols = cols.max(1) as usize;

    let mut line = format!("{} sec", secs);
    line.push_str("    ");
    let line: String = line.chars().take(cols).collect();

    let mut stdout = io::stdout().lock();
    execute!(
        stdout,
        MoveTo(0, y),
        SetBackgroundColor(app.ui_palette.toast.crossterm_bg()),
        SetForegroundColor(app.ui_palette.toast.crossterm_fg()),
        Print(&line),
    )?;
    execute!(stdout, ResetColor)?;
    stdout.flush()?;
    Ok(())
}
