//! Subshell lifecycle helpers used by the main run loop (spawn, cwd sync).

use std::io;
use std::path::PathBuf;

use crate::app::state::AppState;
use crate::core::location::PanelLocation;
use crate::shell::subshell;

pub(crate) fn get_or_create_subshell<'a>(
    subshell: &'a mut Option<subshell::Subshell>,
    cwd: &str,
) -> io::Result<&'a subshell::Subshell> {
    loop {
        match subshell {
            Some(s) => return Ok(s),
            None => {
                *subshell = Some(subshell::Subshell::spawn(cwd)?);
            }
        }
    }
}

/// When setting is on, sync active panel to shell's cwd if it changed (after Ctrl+O or RunCommand return).
/// After any subshell relay, refresh whether the PTY foreground is effectively root (for menu chrome).
pub(crate) fn sync_subshell_root_ui_flag(
    app: &mut AppState,
    sub: Option<&subshell::Subshell>,
) {
    app.subshell_pty_foreground_is_root = sub
        .map(|s| s.pty_foreground_has_root_euid())
        .unwrap_or(false);
}

pub(crate) fn maybe_sync_panel_to_shell_cwd(
    app: &mut AppState,
    shell_cwd: Option<PathBuf>,
) {
    if !app.persisted_settings.sync_panel_to_shell_cwd {
        return;
    }
    let cwd = match shell_cwd {
        Some(c) => c,
        None => return,
    };
    if !app.current_location().is_fs() {
        return;
    }
    let path = std::path::Path::new(&cwd);
    if !path.is_dir() {
        return;
    }
    let shell_canonical = path.canonicalize().unwrap_or(cwd);
    let panel_canonical = app
        .current_location()
        .as_fs_path()
        .and_then(|p| std::fs::canonicalize(p).ok());
    if panel_canonical.as_ref() != Some(&shell_canonical) {
        let _ = app
            .active_panel_mut()
            .navigate_to_location(PanelLocation::fs(shell_canonical));
    }
}
