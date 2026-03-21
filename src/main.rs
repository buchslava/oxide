use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

mod app_state;
mod archive_dialog;
mod clipboard;
mod core;
mod dialog_layout;
mod editor;
mod events;
mod find_dialog;
mod help_dialog;
mod mkdir_dialog;
mod new_file_dialog;
mod panel;
mod panel_overlay;
mod panel_overlay_state;
mod rename_attr;
mod settings_dialog;
mod size_info_dialog;
mod styles;
mod subshell;
mod text_input;
mod ui;
mod util;
mod viewer;

use crate::core::file_ops::FileOperations;
use app_state::{
    AppState, ArchiveMessage, CopyInProgress, CopyProgress, Focus, Operation,
    RenameAttrDialogState, RenameAttrField, SizeInfoDialogState, SizeInfoProgress,
};
use editor::{apply_confirm_choice, close, open_editor, save};
use events::{AppAction, CopyErrorChoice, DeleteConfirmChoice, EventHandler, SettingChange};
use panel::{Panel, PanelOperations, ViewMode};
use std::sync::atomic::Ordering;
use ui::Renderer;
use viewer::{close_viewer, open_viewer, poll_viewer_loading};

/// Log to stderr if the result is an error; otherwise ignore. Use for non-fatal I/O (e.g. save settings, refresh).
fn log_if_err(
    context: &str,
    res: io::Result<()>,
) {
    if let Err(e) = res {
        eprintln!("{}: {}", context, e);
    }
}

fn reset_terminal_character_set_and_modes<W: Write>(out: &mut W) -> io::Result<()> {
    // Defensive terminal reset after shell relay/commands:
    // - ESC ( B / ESC ) B: ASCII G0/G1 (undo DEC special graphics)
    // - SGR reset + ensure wrap is enabled
    out.write_all(b"\x1b(B\x1b)B\x1b[0m\x1b[?7h")?;
    out.flush()?;
    Ok(())
}

fn log_refresh_panel_restore(
    panel: &mut Panel,
    context: &str,
    preferred_after: Option<&str>,
    preferred_before: Option<&str>,
    panel_height: Option<usize>,
) {
    log_if_err(
        context,
        panel.refresh_files_restore_selection(preferred_after, preferred_before, panel_height),
    );
}

/// Restore source panel selection after copy/move/delete and refresh both panels.
fn restore_source_panel_and_refresh(
    app: &mut AppState,
    source_dir: &str,
    restore_after: Option<&str>,
    restore_before: Option<&str>,
) {
    let panel_height = Some(util::compute_panel_height());
    let is_left_source = app.left_panel().get_current_dir() == source_dir;
    if is_left_source {
        log_refresh_panel_restore(
            app.left_panel_mut(),
            "Refresh left panel",
            restore_after,
            restore_before,
            panel_height,
        );
        log_refresh_panel_restore(
            app.right_panel_mut(),
            "Refresh right panel",
            None,
            None,
            panel_height,
        );
    } else {
        log_refresh_panel_restore(
            app.right_panel_mut(),
            "Refresh right panel",
            restore_after,
            restore_before,
            panel_height,
        );
        log_refresh_panel_restore(
            app.left_panel_mut(),
            "Refresh left panel",
            None,
            None,
            panel_height,
        );
    }
}

fn refresh_both_panels_restore_selection(
    app: &mut AppState,
    left_preferred: Option<&str>,
    right_preferred: Option<&str>,
) {
    log_if_err(
        "Refresh panels",
        app.left_panel_mut()
            .refresh_files_restore_selection(left_preferred, None, None),
    );
    log_if_err(
        "Refresh panels",
        app.right_panel_mut()
            .refresh_files_restore_selection(right_preferred, None, None),
    );
}

fn refresh_both_panels_full(app: &mut AppState) {
    refresh_both_panels_restore_selection(app, None, None);
}

fn cycle_view_one_two(view: &mut String) {
    *view = if view.as_str() == "one" {
        "two".to_string()
    } else {
        "one".to_string()
    };
}

fn target_fs_dir_for_copy_params(params: &app_state::CopyParams) -> &Path {
    params
        .target_fs_path
        .as_deref()
        .unwrap_or_else(|| Path::new(&params.target_dir))
}

fn source_item_display_path(
    params: &app_state::CopyParams,
    name: &str,
) -> String {
    params
        .source_location
        .as_ref()
        .map(|loc| crate::core::panel_backend::join_path_display(loc, name))
        .unwrap_or_else(|| {
            FileOperations::join_path(&params.source_dir, name)
                .to_string_lossy()
                .to_string()
        })
}

fn target_display_for_copy_progress(
    operation: Operation,
    params: &app_state::CopyParams,
) -> String {
    match operation {
        Operation::Copy | Operation::Move => params
            .target_location
            .as_ref()
            .map(|loc| loc.display_string())
            .unwrap_or_else(|| params.target_dir.trim_end_matches('/').to_string()),
        Operation::Delete => String::new(),
    }
}

fn apply_panel_location_copy_step(
    c: &CopyInProgress,
    source_loc: &crate::core::location::PanelLocation,
    name: &str,
    is_dir: bool,
    current_path: &str,
) -> (bool, Option<String>, Option<String>) {
    match c.params.target_location.as_ref() {
        Some(crate::core::location::PanelLocation::Zip {
            archive,
            path_inside,
        }) => run_copy_step_into_archive(
            source_loc,
            name,
            is_dir,
            current_path,
            archive,
            path_inside,
            c.operation,
            c.ignore_all_errors,
        ),
        _ => {
            let target_dir = target_fs_dir_for_copy_params(&c.params);
            run_copy_step_backend(
                source_loc,
                name,
                is_dir,
                current_path,
                target_dir,
                c.operation,
                c.overwrite_all,
                c.skip_all,
                c.ignore_all_errors,
            )
        }
    }
}

fn poll_pending_directory_delete(
    app: &mut AppState,
    c: &mut CopyInProgress,
    current_path: &str,
) -> bool {
    let Some(rx) = app.delete_pending_rx.take() else {
        return false;
    };
    match rx.try_recv() {
        Ok(Ok(())) => {
            c.current_index += 1;
            true
        }
        Ok(Err(e)) => {
            if c.ignore_all_errors {
                c.current_index += 1;
            } else {
                app.copy_error_dialog = Some(app_state::CopyErrorState::new(
                    c.operation,
                    format!("{}: {}", current_path, e),
                ));
                app.copy_error_focus = 0;
            }
            true
        }
        Err(mpsc::TryRecvError::Empty) => {
            app.delete_pending_rx = Some(rx);
            true
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            if !c.ignore_all_errors {
                app.copy_error_dialog = Some(app_state::CopyErrorState::new(
                    c.operation,
                    format!("{}: delete failed", current_path),
                ));
                app.copy_error_focus = 0;
            } else {
                c.current_index += 1;
            }
            true
        }
    }
}

fn spawn_background_directory_delete(
    app: &mut AppState,
    source_dir: String,
    entry_name: String,
) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::core::copy_ops::delete_item(
            &source_dir,
            &entry_name,
            true,
        ));
    });
    app.delete_pending_rx = Some(rx);
}

fn run_copy_step_legacy_copy_move(
    app: &mut AppState,
    c: &mut CopyInProgress,
    name: &str,
    is_dir: bool,
) {
    let target_path = FileOperations::join_path(&c.params.target_dir, name);
    if target_path.exists() && !c.overwrite_all && !c.skip_all {
        app.copy_overwrite_dialog = Some(name.to_string());
        app.copy_overwrite_focus = 0;
        return;
    }
    let do_op = |op: Operation| {
        if op == Operation::Move {
            crate::core::copy_ops::move_item(
                &c.params.source_dir,
                &c.params.target_dir,
                name,
                is_dir,
            )
        } else {
            crate::core::copy_ops::copy_item(
                &c.params.source_dir,
                &c.params.target_dir,
                name,
                is_dir,
            )
        }
    };
    if target_path.exists() && c.skip_all {
        c.current_index += 1;
        return;
    }
    if target_path.exists() && c.overwrite_all {
        let result = do_op(c.operation);
        if apply_copy_item_io_outcome(app, c, name, result) {
            c.current_index += 1;
        }
        return;
    }
    if !target_path.exists() {
        let result = do_op(c.operation);
        if apply_copy_item_io_outcome(app, c, name, result) {
            c.current_index += 1;
        }
    }
}

fn run_copy_step_legacy_delete(
    app: &mut AppState,
    c: &mut CopyInProgress,
    name: &str,
    is_dir: bool,
    current_path: &str,
) {
    if poll_pending_directory_delete(app, c, current_path) {
        return;
    }
    if is_dir {
        spawn_background_directory_delete(app, c.params.source_dir.clone(), name.to_string());
        return;
    }
    match crate::core::copy_ops::delete_item(&c.params.source_dir, name, is_dir) {
        Ok(()) => c.current_index += 1,
        Err(e) => {
            if c.ignore_all_errors {
                c.current_index += 1;
            } else {
                app.copy_error_dialog = Some(app_state::CopyErrorState::new(
                    c.operation,
                    format!("{}: {}", current_path, e),
                ));
                app.copy_error_focus = 0;
            }
        }
    }
}

/// Start a copy/move operation and clear overwrite/error dialogs.
fn start_copy_operation(
    app: &mut AppState,
    operation: Operation,
    params: app_state::CopyParams,
) {
    let total = params.items.len();
    let initial_path = params
        .items
        .first()
        .map(|(name, _)| {
            if let Some(loc) = params.source_location.as_ref() {
                crate::core::panel_backend::join_path_display(loc, name)
            } else {
                FileOperations::join_path(&params.source_dir, name)
                    .to_string_lossy()
                    .to_string()
            }
        })
        .unwrap_or_default();

    let target_path = match operation {
        Operation::Copy | Operation::Move => params
            .target_location
            .as_ref()
            .map(|loc| loc.display_string())
            .unwrap_or_else(|| params.target_dir.trim_end_matches('/').to_string()),
        Operation::Delete => String::new(),
    };

    app.copy_in_progress = Some(CopyInProgress {
        operation,
        params,
        current_index: 0,
        overwrite_all: false,
        skip_all: false,
        ignore_all_errors: false,
    });
    app.copy_progress = Some(CopyProgress {
        operation,
        current_path: initial_path,
        target_path,
        current: 1,
        total,
    });
    app.copy_overwrite_dialog = None;
    app.copy_error_dialog = None;
}

fn get_or_create_subshell<'a>(
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
fn maybe_sync_panel_to_shell_cwd(
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
    if !app.get_current_location().is_fs() {
        return;
    }
    let path = std::path::Path::new(&cwd);
    if !path.is_dir() {
        return;
    }
    let shell_canonical = path.canonicalize().unwrap_or(cwd);
    let panel_canonical = app
        .get_current_location()
        .as_fs_path()
        .and_then(|p| std::fs::canonicalize(p).ok());
    if panel_canonical.as_ref() != Some(&shell_canonical) {
        let _ = app
            .active_panel_mut()
            .navigate_to_location(crate::core::location::PanelLocation::fs(shell_canonical));
    }
}

/// One step of copy/move/delete when source is PanelLocation (Fs or Zip). Returns (advance, overwrite_name, error_message).
fn run_copy_step_backend(
    source_loc: &crate::core::location::PanelLocation,
    name: &str,
    is_dir: bool,
    current_path: &str,
    target_dir: &std::path::Path,
    operation: Operation,
    overwrite_all: bool,
    skip_all: bool,
    ignore_all_errors: bool,
) -> (bool, Option<String>, Option<String>) {
    let target_path = target_dir.join(name.trim_end_matches('/'));

    if operation == Operation::Delete {
        let items = &[(name.to_string(), is_dir)];
        return match crate::core::panel_backend::delete_items(source_loc, items) {
            Ok(()) => (true, None, None),
            Err(e) => (
                ignore_all_errors,
                None,
                Some(format!("{}: {}", current_path, e)),
            ),
        };
    }

    if target_path.exists() && !overwrite_all && !skip_all {
        return (false, Some(name.to_string()), None);
    }
    if target_path.exists() && skip_all {
        return (true, None, None);
    }

    let items = &[(name.to_string(), is_dir)];
    let result = if operation == Operation::Move {
        crate::core::panel_backend::move_items_to_fs(source_loc, items, target_dir)
    } else {
        crate::core::panel_backend::copy_items_to_fs(source_loc, items, target_dir)
    };

    match result {
        Ok(()) => (true, None, None),
        Err(e) => (
            ignore_all_errors,
            None,
            Some(format!("{} -> {}: {}", current_path, name, e)),
        ),
    }
}

/// One step of copy/move when target is inside a ZIP. No overwrite dialog; existing entries are overwritten.
fn run_copy_step_into_archive(
    source_loc: &crate::core::location::PanelLocation,
    name: &str,
    is_dir: bool,
    current_path: &str,
    archive_path: &std::path::Path,
    path_inside: &str,
    operation: Operation,
    ignore_all_errors: bool,
) -> (bool, Option<String>, Option<String>) {
    let items = &[(name.to_string(), is_dir)];
    let result = if operation == Operation::Move {
        crate::core::panel_backend::move_items_into_archive(
            source_loc,
            items,
            archive_path,
            path_inside,
        )
    } else {
        crate::core::panel_backend::copy_items_into_archive(
            source_loc,
            items,
            archive_path,
            path_inside,
        )
    };
    match result {
        Ok(()) => (true, None, None),
        Err(e) => (
            ignore_all_errors,
            None,
            Some(format!("{} -> (archive): {}", current_path, e)),
        ),
    }
}

/// Advance the in-progress copy/move by one item. If target exists and no overwrite_all/skip_all, shows overwrite dialog.
fn run_copy_step(app: &mut AppState) {
    let Some(active_copy) = app.copy_in_progress.as_ref() else {
        return;
    };
    let total = active_copy.params.items.len();
    if active_copy.current_index >= total {
        let source_dir = active_copy.params.source_dir.clone();
        let restore_after = active_copy.params.restore_selection_after.clone();
        let restore_before = active_copy.params.restore_selection_before.clone();
        app.copy_in_progress = None;
        app.copy_progress = None;
        app.delete_pending_rx = None;
        app.source_panel_restore = Some((source_dir, restore_after, restore_before));
        return;
    }

    let (name, is_dir, current_path, use_panel_backend) = {
        let Some(c) = app.copy_in_progress.as_ref() else {
            return;
        };
        let (name, is_dir) = &c.params.items[c.current_index];
        (
            name.clone(),
            *is_dir,
            source_item_display_path(&c.params, name),
            c.params.source_location.is_some(),
        )
    };

    {
        let Some(c) = app.copy_in_progress.as_ref() else {
            return;
        };
        app.copy_progress = Some(CopyProgress {
            operation: c.operation,
            current_path: current_path.clone(),
            target_path: target_display_for_copy_progress(c.operation, &c.params),
            current: c.current_index + 1,
            total,
        });
    }

    if use_panel_backend {
        let (advance, overwrite_name, error_msg) = {
            let Some(c) = app.copy_in_progress.as_ref() else {
                return;
            };
            let Some(source_loc) = c.params.source_location.as_ref() else {
                return;
            };
            apply_panel_location_copy_step(c, source_loc, &name, is_dir, &current_path)
        };
        let Some(c) = app.copy_in_progress.as_mut() else {
            return;
        };
        if let Some(n) = overwrite_name {
            app.copy_overwrite_dialog = Some(n);
            app.copy_overwrite_focus = 0;
            return;
        }
        if let Some(msg) = error_msg {
            if !advance {
                app.copy_error_dialog = Some(app_state::CopyErrorState::new(c.operation, msg));
                app.copy_error_focus = 0;
            }
        }
        if advance {
            c.current_index += 1;
        }
        return;
    }

    let mut c = match app.copy_in_progress.take() {
        Some(c) => c,
        None => return,
    };
    if c.operation == Operation::Delete {
        run_copy_step_legacy_delete(app, &mut c, &name, is_dir, &current_path);
    } else {
        run_copy_step_legacy_copy_move(app, &mut c, &name, is_dir);
    }
    app.copy_in_progress = Some(c);
}

fn perform_copy_overwrite_item_io(
    c: &mut CopyInProgress,
    name: &str,
    is_dir: bool,
) -> io::Result<()> {
    if let Some(ref loc) = c.params.source_location {
        let items = &[(name.to_string(), is_dir)];
        match c.params.target_location.as_ref() {
            Some(crate::core::location::PanelLocation::Zip {
                archive,
                path_inside,
            }) => {
                if c.operation == Operation::Move {
                    crate::core::panel_backend::move_items_into_archive(
                        loc,
                        items,
                        archive,
                        path_inside,
                    )
                } else {
                    crate::core::panel_backend::copy_items_into_archive(
                        loc,
                        items,
                        archive,
                        path_inside,
                    )
                }
            }
            _ => {
                let target = target_fs_dir_for_copy_params(&c.params);
                if c.operation == Operation::Move {
                    crate::core::panel_backend::move_items_to_fs(loc, items, target)
                } else {
                    crate::core::panel_backend::copy_items_to_fs(loc, items, target)
                }
            }
        }
    } else if c.operation == Operation::Move {
        crate::core::copy_ops::move_item(&c.params.source_dir, &c.params.target_dir, name, is_dir)
    } else {
        crate::core::copy_ops::copy_item(&c.params.source_dir, &c.params.target_dir, name, is_dir)
    }
}

fn apply_copy_item_io_outcome(
    app: &mut AppState,
    progress: &CopyInProgress,
    name: &str,
    result: io::Result<()>,
) -> bool {
    match result {
        Ok(()) => true,
        Err(e) => {
            if progress.ignore_all_errors {
                true
            } else {
                app.copy_error_dialog = Some(app_state::CopyErrorState::new(
                    progress.operation,
                    format!("{} -> {}: {}", progress.params.source_dir, name, e),
                ));
                app.copy_error_focus = 0;
                false
            }
        }
    }
}

fn finish_copy_if_no_items_left(app: &mut AppState) {
    let should_finish = match app.copy_in_progress.as_ref() {
        Some(c) => c.current_index >= c.params.items.len(),
        None => return,
    };
    if !should_finish {
        return;
    }
    let (source_dir, restore_after, restore_before) = {
        let Some(c) = app.copy_in_progress.as_ref() else {
            return;
        };
        (
            c.params.source_dir.clone(),
            c.params.restore_selection_after.clone(),
            c.params.restore_selection_before.clone(),
        )
    };
    app.copy_in_progress = None;
    app.copy_progress = None;
    app.delete_pending_rx = None;
    restore_source_panel_and_refresh(
        app,
        &source_dir,
        restore_after.as_deref(),
        restore_before.as_deref(),
    );
}

fn handle_copy_overwrite_choice(
    app: &mut AppState,
    n: usize,
) {
    if n == 5 {
        app.copy_in_progress = None;
        app.copy_progress = None;
        app.delete_pending_rx = None;
        refresh_both_panels_full(app);
    } else if let Some(mut c) = app.copy_in_progress.take() {
        let idx = c.current_index;
        if idx < c.params.items.len() {
            let (name, is_dir) = c.params.items[idx].clone();
            let mut advance = false;
            match n {
                1 => {
                    let result = perform_copy_overwrite_item_io(&mut c, &name, is_dir);
                    advance = apply_copy_item_io_outcome(app, &c, &name, result);
                }
                2 => {
                    c.overwrite_all = true;
                    let result = perform_copy_overwrite_item_io(&mut c, &name, is_dir);
                    advance = apply_copy_item_io_outcome(app, &c, &name, result);
                }
                3 => advance = true,
                4 => {
                    c.skip_all = true;
                    advance = true;
                }
                _ => {}
            }
            if advance {
                c.current_index += 1;
            }
        }
        app.copy_in_progress = Some(c);
    }
    app.copy_overwrite_dialog = None;
    finish_copy_if_no_items_left(app);
}

fn apply_persisted_setting_change(
    app: &mut AppState,
    change: SettingChange,
) {
    match change {
        SettingChange::AutosaveToggle => {
            app.persisted_settings.autosave = !app.persisted_settings.autosave;
        }
        SettingChange::SyncPanelToShellCwdToggle => {
            app.persisted_settings.sync_panel_to_shell_cwd =
                !app.persisted_settings.sync_panel_to_shell_cwd;
        }
        SettingChange::AutoReopenPanelsAfterCommandToggle => {
            app.persisted_settings.auto_reopen_panels_after_command =
                !app.persisted_settings.auto_reopen_panels_after_command;
        }
        SettingChange::AutoReopenPanelsAfterCommandDelayCycle => {
            let cur = app
                .persisted_settings
                .auto_reopen_panels_after_command_delay_secs
                .max(1);
            let max = 30u64;
            let next = if cur >= max { 1 } else { cur + 1 };
            app.persisted_settings
                .auto_reopen_panels_after_command_delay_secs = next;
        }
        SettingChange::LeftViewCycle => cycle_view_one_two(&mut app.persisted_settings.left_view),
        SettingChange::RightViewCycle => cycle_view_one_two(&mut app.persisted_settings.right_view),
        SettingChange::LeftShowHiddenToggle => {
            app.persisted_settings.left_show_hidden = !app.persisted_settings.left_show_hidden;
        }
        SettingChange::RightShowHiddenToggle => {
            app.persisted_settings.right_show_hidden = !app.persisted_settings.right_show_hidden;
        }
        SettingChange::LeftSortCycle => {
            app.persisted_settings.left_sort =
                crate::core::file_ops::cycle_sort_mode(&app.persisted_settings.left_sort, true);
        }
        SettingChange::RightSortCycle => {
            app.persisted_settings.right_sort =
                crate::core::file_ops::cycle_sort_mode(&app.persisted_settings.right_sort, true);
        }
        SettingChange::LeftSortCyclePrev => {
            app.persisted_settings.left_sort =
                crate::core::file_ops::cycle_sort_mode(&app.persisted_settings.left_sort, false);
        }
        SettingChange::RightSortCyclePrev => {
            app.persisted_settings.right_sort =
                crate::core::file_ops::cycle_sort_mode(&app.persisted_settings.right_sort, false);
        }
        SettingChange::LeftDirsFirstToggle => {
            app.persisted_settings.left_dirs_first = !app.persisted_settings.left_dirs_first;
        }
        SettingChange::RightDirsFirstToggle => {
            app.persisted_settings.right_dirs_first = !app.persisted_settings.right_dirs_first;
        }
    }
}

fn setting_change_skips_panel_resync(change: SettingChange) -> bool {
    matches!(
        change,
        SettingChange::AutosaveToggle
            | SettingChange::SyncPanelToShellCwdToggle
            | SettingChange::AutoReopenPanelsAfterCommandToggle
            | SettingChange::AutoReopenPanelsAfterCommandDelayCycle
    )
}

fn toggle_show_hidden_on_active_panel(app: &mut AppState) {
    let panel_height = util::compute_panel_height();
    if app.active_panel() == 0 {
        let new_show = !app.left_panel().get_show_hidden();
        app.left_panel_mut().set_show_hidden(new_show);
        app.persisted_settings.left_show_hidden = new_show;
        app.show_hidden_files = new_show;
        let left_name = app.left_panel().get_selected_file().map(|f| f.name.clone());
        log_if_err(
            "Save settings",
            crate::core::settings::save(&app.persisted_settings),
        );
        log_if_err(
            "Refresh panel",
            app.left_panel_mut().refresh_files_restore_selection(
                left_name.as_deref(),
                None,
                Some(panel_height),
            ),
        );
    } else {
        let new_show = !app.right_panel().get_show_hidden();
        app.right_panel_mut().set_show_hidden(new_show);
        app.persisted_settings.right_show_hidden = new_show;
        app.show_hidden_files = new_show;
        let right_name = app
            .right_panel()
            .get_selected_file()
            .map(|f| f.name.clone());
        log_if_err(
            "Save settings",
            crate::core::settings::save(&app.persisted_settings),
        );
        log_if_err(
            "Refresh panel",
            app.right_panel_mut().refresh_files_restore_selection(
                right_name.as_deref(),
                None,
                Some(panel_height),
            ),
        );
    }
}

fn main() -> Result<(), io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app_settings = crate::core::settings::load();
    let _ = crate::core::settings::ensure_config_dir();
    log_if_err(
        "Save settings (startup)",
        crate::core::settings::save(&app_settings),
    );
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let home_str = home.to_string_lossy().to_string();
    let left_cwd = app_settings
        .left_cwd
        .clone()
        .unwrap_or_else(|| home_str.clone());
    let right_cwd = app_settings
        .right_cwd
        .clone()
        .unwrap_or_else(|| home_str.clone());
    let mut app = AppState::new_with_initial(left_cwd, right_cwd, &home_str, app_settings)?;
    let mut subshell: Option<subshell::Subshell> = None;

    // Process any events from EnterAlternateScreen (never discard keys).
    if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
            DisableBracketedPaste
        )?;
        terminal.show_cursor()?;
        return Ok(());
    }
    // Show first frame; brief delay then process queue so first keypress is handled, not discarded.
    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
    std::thread::sleep(std::time::Duration::from_millis(50));
    if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
            DisableBracketedPaste
        )?;
        terminal.show_cursor()?;
        return Ok(());
    }

    // Preload subshell so first Ctrl+O has no spawn delay (MC inits subshell at startup).
    let _ = get_or_create_subshell(&mut subshell, app.get_current_dir());

    // Software blinking for command-line cursor (terminal-native blink is not reliable everywhere).
    let mut cmd_cursor_blink_visible = true;
    let mut cmd_cursor_blink_last_toggle = std::time::Instant::now();

    loop {
        terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;

        let find_input_focused = app.find_dialog.as_ref().map_or(false, |d| {
            use crate::app_state::FindDialogPhase;
            d.phase == FindDialogPhase::Parameter && d.focus <= 2
        });
        let mkdir_input_focused = app.mkdir_dialog.as_ref().map_or(false, |d| d.focus == 0);
        let archive_input_focused = app.archive_dialog.as_ref().map_or(false, |d| d.focus == 0);
        let new_file_input_focused = app.new_file_dialog.as_ref().map_or(false, |d| d.focus == 0);
        let rename_name_focused = matches!(
            app.rename_attr_dialog.as_ref(),
            Some(RenameAttrDialogState::Single {
                focus: RenameAttrField::Name,
                ..
            })
        );
        let input_cursor_blink = find_input_focused
            || mkdir_input_focused
            || archive_input_focused
            || new_file_input_focused
            || rename_name_focused;
        let show_cursor = app.editor_screen.is_some()
            || mkdir_input_focused
            || archive_input_focused
            || new_file_input_focused
            || rename_name_focused
            || (app.focus == Focus::CommandLine)
            || find_input_focused;
        let command_line_cursor_active = app.focus == Focus::CommandLine
            && app.editor_screen.is_none()
            && app.mkdir_dialog.is_none()
            && app.archive_dialog.is_none()
            && app.new_file_dialog.is_none();

        if command_line_cursor_active || input_cursor_blink {
            if cmd_cursor_blink_last_toggle.elapsed() >= std::time::Duration::from_millis(500) {
                cmd_cursor_blink_visible = !cmd_cursor_blink_visible;
                cmd_cursor_blink_last_toggle = std::time::Instant::now();
            }
        } else {
            // Reset blink state when leaving so cursor appears immediately on next focus.
            cmd_cursor_blink_visible = true;
            cmd_cursor_blink_last_toggle = std::time::Instant::now();
        }

        if show_cursor {
            if command_line_cursor_active || input_cursor_blink {
                if cmd_cursor_blink_visible {
                    let _ = terminal.show_cursor();
                } else {
                    let _ = crossterm::execute!(terminal.backend_mut(), crossterm::cursor::Hide);
                }
            } else {
                let _ = terminal.show_cursor();
            }
        } else {
            let _ = crossterm::execute!(terminal.backend_mut(), crossterm::cursor::Hide);
        }

        if app.copy_in_progress.is_some()
            && app.copy_overwrite_dialog.is_none()
            && app.copy_error_dialog.is_none()
        {
            run_copy_step(&mut app);
            if app.copy_overwrite_dialog.is_some()
                || app.copy_error_dialog.is_some()
                || app.copy_in_progress.is_none()
            {
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
            }
        }

        // Poll viewer file loading (background thread); Esc stays responsive for large files.
        if poll_viewer_loading(&mut app) {
            terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
        }

        // Poll size info calculation progress (background thread).
        if let Some(rx) = app.size_info_pending_rx.take() {
            match rx.try_recv() {
                Ok(SizeInfoProgress::Progress {
                    current,
                    total,
                    total_bytes,
                    file_count,
                    dir_count,
                }) => {
                    app.size_info_dialog = Some(SizeInfoDialogState::Calculating {
                        current,
                        total,
                        total_bytes,
                        file_count,
                        dir_count,
                    });
                    app.size_info_pending_rx = Some(rx);
                    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                }
                Ok(SizeInfoProgress::Done {
                    total_bytes,
                    file_count,
                    dir_count,
                }) => {
                    app.size_info_dialog = Some(SizeInfoDialogState::Done {
                        total_bytes,
                        file_count,
                        dir_count,
                    });
                }
                Err(mpsc::TryRecvError::Empty) => {
                    app.size_info_pending_rx = Some(rx);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    app.size_info_dialog = None;
                }
            }
        }

        // Poll archive progress (background thread).
        if let Some(rx) = app.archive_pending_rx.take() {
            match rx.try_recv() {
                Ok(ArchiveMessage::Progress(p)) => {
                    app.archive_progress = Some(p);
                    app.archive_pending_rx = Some(rx);
                    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                }
                Ok(ArchiveMessage::Done(res, name_for_selection)) => {
                    app.archive_progress = None;
                    app.archive_cancel = None;
                    if let Err(e) = res {
                        if e.kind() != std::io::ErrorKind::Interrupted {
                            eprintln!("Archive error: {}", e);
                        }
                    } else {
                        let panel_height = util::compute_panel_height();
                        let _ = app.active_panel_mut().refresh_files_restore_selection(
                            name_for_selection.as_deref(),
                            None,
                            Some(panel_height),
                        );
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    app.archive_pending_rx = Some(rx);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    app.archive_progress = None;
                    app.archive_cancel = None;
                }
            }
        }

        // After copy/move/delete completes: restore source panel selection (file after, else file before) and scroll.
        if let Some((source_dir, restore_after, restore_before)) = app.source_panel_restore.take() {
            restore_source_panel_and_refresh(
                &mut app,
                &source_dir,
                restore_after.as_deref(),
                restore_before.as_deref(),
            );
            terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
        }

        match EventHandler::handle_events(&mut app)? {
            AppAction::Quit => {
                app.maybe_persist_panel_dirs();
                break;
            }
            AppAction::CopyOverwriteChoice(n) => {
                handle_copy_overwrite_choice(&mut app, n);
            }
            AppAction::CopyErrorChoice(choice) => {
                app.copy_error_dialog = None;
                match choice {
                    CopyErrorChoice::Ignore => {
                        if let Some(ref mut c) = app.copy_in_progress {
                            c.current_index += 1;
                        }
                    }
                    CopyErrorChoice::IgnoreAll => {
                        if let Some(ref mut c) = app.copy_in_progress {
                            c.ignore_all_errors = true;
                            c.current_index += 1;
                        }
                    }
                    CopyErrorChoice::Cancel => {
                        app.copy_in_progress = None;
                        app.copy_progress = None;
                        app.delete_pending_rx = None;
                        refresh_both_panels_full(&mut app);
                    }
                }
            }
            AppAction::CopyCancel => {
                app.copy_in_progress = None;
                app.copy_progress = None;
                app.copy_overwrite_dialog = None;
                app.copy_error_dialog = None;
                app.delete_pending_rx = None;
                refresh_both_panels_full(&mut app);
            }
            AppAction::ArchiveProgressCancel => {
                if let Some(c) = app.archive_cancel.take() {
                    c.store(true, Ordering::Relaxed);
                }
                app.archive_progress = None;
                app.archive_pending_rx = None;
            }
            AppAction::DeleteConfirmChoice(choice) => {
                let pending = app.operation_confirm_pending.take();
                if let (DeleteConfirmChoice::Yes, Some((op, params))) = (choice, pending) {
                    let skip_same_dir = matches!(op, Operation::Copy | Operation::Move)
                        && params.source_dir == params.target_dir;
                    if !skip_same_dir {
                        start_copy_operation(&mut app, op, params);
                    }
                }
            }
            AppAction::OpenViewer => {
                open_viewer(&mut app);
            }
            AppAction::ViewerClose => {
                close_viewer(&mut app);
                if app.find_dialog.is_some() {
                    app.focus = crate::app_state::Focus::FindDialog;
                }
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
            }
            AppAction::OpenEditor => {
                open_editor(&mut app);
            }
            AppAction::EditorSave => save(&mut app),
            AppAction::EditorClose => {
                close(&mut app);
                if app.find_dialog.is_some() {
                    app.focus = crate::app_state::Focus::FindDialog;
                }
            }
            AppAction::EditorConfirmChoice(choice) => apply_confirm_choice(&mut app, choice),
            AppAction::OpenMkdirDialog => mkdir_dialog::open(&mut app),
            AppAction::MkdirConfirm => {
                if let Some(name) = mkdir_dialog::confirm(&mut app) {
                    let name = name.trim();
                    if !name.is_empty() {
                        mkdir_dialog::create_and_refresh(&mut app, name);
                    }
                }
            }
            AppAction::MkdirCancel => mkdir_dialog::cancel(&mut app),
            AppAction::OpenArchiveDialog => archive_dialog::open(&mut app),
            AppAction::ArchiveConfirm => {
                let (items, ..) = app
                    .active_panel_ref()
                    .get_names_to_copy_with_restore_neighbors();
                if let Some(name) = archive_dialog::confirm(&mut app) {
                    let name = name.trim();
                    if !name.is_empty() && !items.is_empty() {
                        archive_dialog::start_archive_background(&mut app, name, &items);
                    }
                }
            }
            AppAction::ArchiveCancel => archive_dialog::cancel(&mut app),
            AppAction::OpenNewFileDialog => new_file_dialog::open(&mut app),
            AppAction::NewFileConfirm => {
                if let Some(name) = new_file_dialog::confirm(&mut app) {
                    let name = name.trim();
                    if !name.is_empty() {
                        new_file_dialog::create_and_refresh(&mut app, name);
                    }
                }
            }
            AppAction::NewFileCancel => new_file_dialog::cancel(&mut app),
            AppAction::OpenRenameAttrDialog => rename_attr::open(&mut app),
            AppAction::OpenSizeInfoDialog => size_info_dialog::open(&mut app),
            AppAction::SizeInfoClose => size_info_dialog::close(&mut app),
            AppAction::OpenHelpDialog => help_dialog::open(&mut app),
            AppAction::HelpClose => help_dialog::close(&mut app),
            AppAction::OpenSettingsDialog => settings_dialog::open(&mut app),
            AppAction::SettingsClose => settings_dialog::close(&mut app),
            AppAction::OpenLeftPanelSettings => panel_overlay::open_left(&mut app),
            AppAction::OpenRightPanelSettings => panel_overlay::open_right(&mut app),
            AppAction::CloseLeftPanelSettings => panel_overlay::close_left(&mut app),
            AppAction::CloseRightPanelSettings => panel_overlay::close_right(&mut app),
            AppAction::OpenFindDialog => find_dialog::open(&mut app),
            AppAction::FindClose => find_dialog::close(&mut app),
            AppAction::FindStartSearch => find_dialog::start_search(&mut app),
            AppAction::FindChdir => {
                if let Some(ref d) = app.find_dialog {
                    let rows = find_dialog::build_display_rows(&d.results);
                    if let Some(row) = rows.get(d.selected_index) {
                        match row {
                            find_dialog::FindDisplayRow::Folder(path) => {
                                let loc = crate::core::location::PanelLocation::fs(path);
                                if app.active_panel_mut().navigate_to_location(loc).is_ok() {}
                            }
                            find_dialog::FindDisplayRow::File(r) => {
                                if let (Some(parent), Some(name)) = (
                                    r.path.parent(),
                                    r.path.file_name().map(|n| n.to_string_lossy().to_string()),
                                ) {
                                    let loc = crate::core::location::PanelLocation::fs(parent);
                                    let panel_height = util::compute_panel_height();
                                    if app.active_panel_mut().navigate_to_location(loc).is_ok() {
                                        log_if_err(
                                            "Refresh panel",
                                            app.active_panel_mut().refresh_files_restore_selection(
                                                Some(name.as_str()),
                                                None,
                                                Some(panel_height),
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                find_dialog::close(&mut app);
            }
            AppAction::FindView => {
                if let Some(ref d) = app.find_dialog {
                    let rows = find_dialog::build_display_rows(&d.results);
                    if let Some(find_dialog::FindDisplayRow::File(r)) = rows.get(d.selected_index) {
                        if r.path.is_file() {
                            viewer::open_viewer_path(&mut app, r.path.clone(), r.line);
                        }
                    }
                }
                // Do not close Find dialog: user returns to it after closing viewer.
            }
            AppAction::FindEdit => {
                if let Some(ref d) = app.find_dialog {
                    let rows = find_dialog::build_display_rows(&d.results);
                    if let Some(find_dialog::FindDisplayRow::File(r)) = rows.get(d.selected_index) {
                        if r.path.is_file() {
                            editor::open_editor_path(&mut app, r.path.clone());
                        }
                    }
                }
                // Do not close Find dialog: user returns to it after closing editor.
            }
            AppAction::PanelNavigated => app.maybe_persist_panel_dirs(),
            AppAction::SettingChange(change) => {
                apply_persisted_setting_change(&mut app, change);
                log_if_err(
                    "Save settings",
                    crate::core::settings::save(&app.persisted_settings),
                );
                if !setting_change_skips_panel_resync(change) {
                    app.sync_from_persisted_settings();
                }
            }
            AppAction::ViewModeToggled => {
                let view = if app.active_panel_ref().get_view_mode() == ViewMode::SingleColumn {
                    "one".to_string()
                } else {
                    "two".to_string()
                };
                if app.active_panel() == 0 {
                    app.persisted_settings.left_view = view;
                } else {
                    app.persisted_settings.right_view = view;
                }
                log_if_err(
                    "Save settings",
                    crate::core::settings::save(&app.persisted_settings),
                );
            }
            AppAction::ToggleShowHidden => {
                toggle_show_hidden_on_active_panel(&mut app);
            }
            AppAction::RenameAttrConfirm => {
                if rename_attr::apply(&mut app) {
                    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                }
            }
            AppAction::RenameAttrCancel => rename_attr::cancel(&mut app),
            AppAction::Suspend => {
                // --- Ctrl+O: hand terminal to subshell. Use single-writer flow (REFERENCE.md "Solution: Second Ctrl+O uglification"): do NOT use backend for leave alternate; write everything to stdout.
                terminal.flush()?;
                let _ = terminal.backend_mut().flush();
                let _ = std::io::stdout().flush();
                let prepared = subshell::Subshell::prepare_for_relay();
                {
                    let mut stdout = std::io::stdout().lock();
                    let _ = subshell::Subshell::write_relay_reset_sequence(&mut stdout);
                    let _ = stdout.flush();
                }
                let shell_cwd =
                    if let Ok(sub) = get_or_create_subshell(&mut subshell, app.get_current_dir()) {
                        let cwd = app.get_current_dir().to_string();
                        let _ = sub.run_cd_then_relay(&cwd, Some(prepared));
                        sub.get_cwd()
                    } else {
                        eprintln!("Subshell error");
                        None
                    };
                let _ = reset_terminal_character_set_and_modes(terminal.backend_mut());
                // 3) Return: enter alternate first (Ratatui), then drain input (MC tty_flush_input), then clear + redraw.
                execute!(
                    terminal.backend_mut(),
                    EnterAlternateScreen,
                    crossterm::cursor::Hide,
                    crossterm::event::EnableMouseCapture
                )?;
                if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                    break;
                }
                app.focus_panel();
                maybe_sync_panel_to_shell_cwd(&mut app, shell_cwd);
                terminal.clear()?;
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                // Do NOT terminal.flush() after draw(): draw() already flushes; extra flush can paint black.
                if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                    break;
                }
            }
            AppAction::Copy(params) => start_copy_operation(&mut app, Operation::Copy, params),
            AppAction::Move(params) => start_copy_operation(&mut app, Operation::Move, params),
            AppAction::RunCommand(cmd) => {
                // Run command in the subshell (MC-style): output and prompt stay visible; Ctrl+O returns to panels. Same single-writer flow as Suspend.
                let auto_exit_after_idle =
                    if app.persisted_settings.auto_reopen_panels_after_command {
                        let secs = app
                            .persisted_settings
                            .auto_reopen_panels_after_command_delay_secs
                            .max(1);
                        Some(std::time::Duration::from_secs(secs))
                    } else {
                        None
                    };
                let left_selected = app
                    .left_panel_mut()
                    .get_selected_file()
                    .map(|f| f.name.to_string());
                let right_selected = app
                    .right_panel_mut()
                    .get_selected_file()
                    .map(|f| f.name.to_string());
                let cwd = app.get_current_dir().to_string();
                terminal.flush()?;
                let _ = terminal.backend_mut().flush();
                let _ = std::io::stdout().flush();
                let prepared = subshell::Subshell::prepare_for_relay();
                {
                    let mut stdout = std::io::stdout().lock();
                    let _ = subshell::Subshell::write_relay_reset_sequence(&mut stdout);
                    let _ = stdout.flush();
                }
                let shell_cwd =
                    if let Ok(sub) = get_or_create_subshell(&mut subshell, app.get_current_dir()) {
                        let relay_outcome = sub.run_command_then_relay(
                            &cwd,
                            &cmd,
                            Some(prepared),
                            auto_exit_after_idle,
                        );
                        let cwd_opt = sub.get_cwd();
                        let _ = relay_outcome;
                        cwd_opt
                    } else {
                        eprintln!("Subshell error");
                        None
                    };
                let _ = reset_terminal_character_set_and_modes(terminal.backend_mut());
                // 3) Return: enter alternate, drain, focus panel, clear + draw (panels visible again).
                execute!(
                    terminal.backend_mut(),
                    EnterAlternateScreen,
                    crossterm::cursor::Hide,
                    crossterm::event::EnableMouseCapture
                )?;
                if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                    break;
                }
                app.focus_panel();
                maybe_sync_panel_to_shell_cwd(&mut app, shell_cwd);
                terminal.clear()?;
                refresh_both_panels_restore_selection(
                    &mut app,
                    left_selected.as_deref(),
                    right_selected.as_deref(),
                );
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                    break;
                }
            }
            AppAction::Continue => {}
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    Ok(())
}
