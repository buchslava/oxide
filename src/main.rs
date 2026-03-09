use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use ratatui::{backend::CrosstermBackend, Terminal};
use crossterm::{
    event::EnableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

mod app_state;
mod util;
mod copy_ops;
mod editor;
mod events;
mod file_ops;
mod location;
mod mkdir_dialog;
mod panel;
mod panel_backend;
mod rename_attr;
mod settings;
mod settings_dialog;
mod size_info_dialog;
mod styles;
mod subshell;
mod ui;
mod viewer;

use app_state::{AppState, CopyInProgress, CopyProgress, Focus, Operation, RenameAttrDialogState, RenameAttrField, SizeInfoDialogState, SizeInfoProgress};
use editor::{apply_confirm_choice, close, open_editor, save};
use events::{EventHandler, AppAction, CopyErrorChoice, DeleteConfirmChoice, SettingChange};
use viewer::{close_viewer, open_viewer, poll_viewer_loading};
use file_ops::FileOperations;
use panel::{PanelOperations, ViewMode};
use ui::Renderer;

fn reset_terminal_character_set_and_modes<W: Write>(out: &mut W) -> io::Result<()> {
    // Defensive terminal reset after shell relay/commands:
    // - ESC ( B / ESC ) B: ASCII G0/G1 (undo DEC special graphics)
    // - SGR reset + ensure wrap is enabled
    out.write_all(b"\x1b(B\x1b)B\x1b[0m\x1b[?7h")?;
    out.flush()?;
    Ok(())
}

/// Restore source panel selection after copy/move/delete and refresh both panels.
fn restore_source_panel_and_refresh(
    app: &mut AppState,
    source_dir: &str,
    restore_after: Option<&str>,
    restore_before: Option<&str>,
) {
    let panel_height = util::compute_panel_height();
    let is_left_source = app.left_panel().get_current_dir() == source_dir;
    if is_left_source {
        let _ = app.left_panel_mut().refresh_files_restore_selection(
            restore_after,
            restore_before,
            Some(panel_height),
        );
        let _ = app.right_panel_mut().refresh_files_restore_selection(None, None, Some(panel_height));
    } else {
        let _ = app.right_panel_mut().refresh_files_restore_selection(
            restore_after,
            restore_before,
            Some(panel_height),
        );
        let _ = app.left_panel_mut().refresh_files_restore_selection(None, None, Some(panel_height));
    }
}

/// Start a copy/move operation and clear overwrite/error dialogs.
fn start_copy_operation(app: &mut AppState, operation: Operation, params: app_state::CopyParams) {
    let total = params.items.len();
    let initial_path = params.items.first().map(|(name, _)| {
        if let Some(loc) = params.source_location.as_ref() {
            panel_backend::join_path_display(loc, name)
        } else {
            FileOperations::join_path(&params.source_dir, name)
                .to_string_lossy()
                .to_string()
        }
    }).unwrap_or_default();

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
        current: 0,
        total,
    });
    app.copy_overwrite_dialog = None;
    app.copy_error_dialog = None;
}

fn get_or_create_subshell<'a>(
    subshell: &'a mut Option<subshell::Subshell>,
    cwd: &str,
) -> io::Result<&'a subshell::Subshell> {
    if subshell.is_none() {
        *subshell = Some(subshell::Subshell::spawn(cwd)?);
    }
    Ok(subshell.as_ref().unwrap())
}

/// One step of copy/move/delete when source is PanelLocation (Fs or Zip). Returns (advance, overwrite_name, error_message).
fn run_copy_step_backend(
    source_loc: &crate::location::PanelLocation,
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
        return match panel_backend::delete_items(source_loc, items) {
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
        panel_backend::move_items_to_fs(source_loc, items, target_dir)
    } else {
        panel_backend::copy_items_to_fs(source_loc, items, target_dir)
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

/// Advance the in-progress copy/move by one item. If target exists and no overwrite_all/skip_all, shows overwrite dialog.
fn run_copy_step(app: &mut AppState) {
    let Some(ref mut c) = app.copy_in_progress else { return };
    let total = c.params.items.len();
    if c.current_index >= total {
        let source_dir = c.params.source_dir.clone();
        let restore_after = c.params.restore_selection_after.clone();
        let restore_before = c.params.restore_selection_before.clone();
        app.copy_in_progress = None;
        app.copy_progress = None;
        app.delete_pending_rx = None;
        app.source_panel_restore = Some((source_dir, restore_after, restore_before));
        return;
    }
    let (name, is_dir) = &c.params.items[c.current_index];
    let current_path = c.params.source_location.as_ref()
        .map(|loc| panel_backend::join_path_display(loc, name))
        .unwrap_or_else(|| FileOperations::join_path(&c.params.source_dir, name).to_string_lossy().to_string());
    app.copy_progress = Some(CopyProgress {
        operation: c.operation,
        current_path: current_path.clone(),
        current: c.current_index + 1,
        total,
    });

    if let Some(ref source_loc) = c.params.source_location {
        let target_dir = c.params.target_fs_path.as_deref()
            .unwrap_or_else(|| std::path::Path::new(&c.params.target_dir));
        let (advance, overwrite_name, error_msg) = run_copy_step_backend(
            source_loc,
            name,
            *is_dir,
            &current_path,
            target_dir,
            c.operation,
            c.overwrite_all,
            c.skip_all,
            c.ignore_all_errors,
        );
        if let Some(n) = overwrite_name {
            app.copy_overwrite_dialog = Some(n);
            app.copy_overwrite_focus = 0;
            return;
        }
        if let Some(msg) = error_msg {
            if !advance {
                app.copy_error_dialog = Some(app_state::CopyErrorState {
                    operation: c.operation,
                    message: msg,
                });
                app.copy_error_focus = 0;
            }
        }
        if advance {
            c.current_index += 1;
        }
        return;
    }

    // Legacy path: copy_ops with filesystem paths only.
    // Delete: no target or overwrite; just remove from source_dir. Run directory delete in background so UI stays responsive.
    if c.operation == Operation::Delete {
        // Poll pending directory delete from previous step
        if let Some(rx) = app.delete_pending_rx.take() {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    c.current_index += 1;
                    return;
                }
                Ok(Err(e)) => {
                    if c.ignore_all_errors {
                        c.current_index += 1;
                    } else {
                        app.copy_error_dialog = Some(app_state::CopyErrorState {
                            operation: c.operation,
                            message: format!("{}: {}", current_path, e),
                        });
                        app.copy_error_focus = 0;
                    }
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    app.delete_pending_rx = Some(rx);
                    return; // still deleting, keep UI responsive
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // thread panicked or dropped; treat as error
                    if !c.ignore_all_errors {
                        app.copy_error_dialog = Some(app_state::CopyErrorState {
                            operation: c.operation,
                            message: format!("{}: delete failed", current_path),
                        });
                        app.copy_error_focus = 0;
                    } else {
                        c.current_index += 1;
                    }
                    return;
                }
            }
        }
        if *is_dir {
            let source_dir = c.params.source_dir.clone();
            let name = name.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(copy_ops::delete_item(&source_dir, &name, true));
            });
            app.delete_pending_rx = Some(rx);
            return;
        }
        if let Err(e) = copy_ops::delete_item(&c.params.source_dir, name, *is_dir) {
            if c.ignore_all_errors {
                c.current_index += 1;
            } else {
                app.copy_error_dialog = Some(app_state::CopyErrorState {
                    operation: c.operation,
                    message: format!("{}: {}", current_path, e),
                });
                app.copy_error_focus = 0;
            }
            return;
        }
        c.current_index += 1;
        return;
    }
    let target_path = FileOperations::join_path(&c.params.target_dir, name);
    if target_path.exists() && !c.overwrite_all && !c.skip_all {
        app.copy_overwrite_dialog = Some(name.clone());
        app.copy_overwrite_focus = 0;
        return;
    }
    let do_op = |op: Operation| {
        if op == Operation::Move {
            copy_ops::move_item(&c.params.source_dir, &c.params.target_dir, name, *is_dir)
        } else {
            copy_ops::copy_item(&c.params.source_dir, &c.params.target_dir, name, *is_dir)
        }
    };
    if target_path.exists() && c.skip_all {
        c.current_index += 1;
        return;
    }
    if target_path.exists() && c.overwrite_all {
        if let Err(e) = do_op(c.operation) {
            if c.ignore_all_errors {
                c.current_index += 1;
            } else {
                app.copy_error_dialog = Some(app_state::CopyErrorState {
                    operation: c.operation,
                    message: format!("{} -> {}: {}", c.params.source_dir, name, e),
                });
                app.copy_error_focus = 0;
            }
            return;
        }
        c.current_index += 1;
        return;
    }
    if !target_path.exists() {
        if let Err(e) = do_op(c.operation) {
            if c.ignore_all_errors {
                c.current_index += 1;
            } else {
                app.copy_error_dialog = Some(app_state::CopyErrorState {
                    operation: c.operation,
                    message: format!("{} -> {}: {}", c.params.source_dir, name, e),
                });
                app.copy_error_focus = 0;
            }
            return;
        }
        c.current_index += 1;
    }
}

fn main() -> Result<(), io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app_settings = settings::load();
    let _ = settings::ensure_config_dir();
    let _ = settings::save(&app_settings);
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
        execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
        terminal.show_cursor()?;
        return Ok(());
    }
    // Show first frame; brief delay then process queue so first keypress is handled, not discarded.
    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
    std::thread::sleep(std::time::Duration::from_millis(50));
    if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
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

        let show_cursor = app.editor_screen.is_some()
            || app.mkdir_dialog.as_ref().map_or(false, |d| d.focus == 0)
            || matches!(
                app.rename_attr_dialog.as_ref(),
                Some(RenameAttrDialogState::Single { focus: RenameAttrField::Name, .. })
            )
            || (app.focus == Focus::CommandLine);
        let command_line_cursor_active = app.focus == Focus::CommandLine
            && app.editor_screen.is_none()
            && app.mkdir_dialog.is_none()
            && app.rename_attr_dialog.is_none();

        if command_line_cursor_active {
            if cmd_cursor_blink_last_toggle.elapsed() >= std::time::Duration::from_millis(500) {
                cmd_cursor_blink_visible = !cmd_cursor_blink_visible;
                cmd_cursor_blink_last_toggle = std::time::Instant::now();
            }
        } else {
            // Reset blink state when leaving command line so it appears immediately on next focus.
            cmd_cursor_blink_visible = true;
            cmd_cursor_blink_last_toggle = std::time::Instant::now();
        }

        if show_cursor {
            if command_line_cursor_active {
                if cmd_cursor_blink_visible {
                    let _ = terminal.show_cursor();
                } else {
                    let _ = crossterm::execute!(terminal.backend_mut(), crossterm::cursor::Hide);
                }
            } else {
                let _ = terminal.show_cursor();
            }
        } else {
            let _ = crossterm::execute!(
                terminal.backend_mut(),
                crossterm::cursor::Hide
            );
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
                if n == 5 {
                    app.copy_in_progress = None;
                    app.copy_progress = None;
                    app.delete_pending_rx = None;
                    let _ = app.left_panel_mut().refresh_files_restore_selection(None, None, None);
                    let _ = app.right_panel_mut().refresh_files_restore_selection(None, None, None);
                } else if let Some(ref mut c) = app.copy_in_progress {
                    let idx = c.current_index;
                    if idx < c.params.items.len() {
                        let (name, is_dir) = c.params.items[idx].clone();
                        let mut advance = false;
                        let do_op = || -> io::Result<()> {
                            if let Some(ref loc) = c.params.source_location {
                                let target = c.params.target_fs_path.as_deref()
                                    .unwrap_or_else(|| std::path::Path::new(&c.params.target_dir));
                                let items = &[(name.clone(), is_dir)];
                                if c.operation == Operation::Move {
                                    panel_backend::move_items_to_fs(loc, items, target)
                                } else {
                                    panel_backend::copy_items_to_fs(loc, items, target)
                                }
                            } else if c.operation == Operation::Move {
                                copy_ops::move_item(
                                    &c.params.source_dir,
                                    &c.params.target_dir,
                                    &name,
                                    is_dir,
                                )
                            } else {
                                copy_ops::copy_item(
                                    &c.params.source_dir,
                                    &c.params.target_dir,
                                    &name,
                                    is_dir,
                                )
                            }
                        };
                        match n {
                            1 => {
                                let result = do_op();
                                if result.is_ok() {
                                    advance = true;
                                } else if let Err(e) = result {
                                    if c.ignore_all_errors {
                                        advance = true;
                                    } else {
                                        app.copy_error_dialog = Some(app_state::CopyErrorState {
                                            operation: c.operation,
                                            message: format!("{} -> {}: {}", c.params.source_dir, name, e),
                                        });
                                        app.copy_error_focus = 0;
                                    }
                                }
                            }
                            2 => {
                                c.overwrite_all = true;
                                let result = do_op();
                                if result.is_ok() {
                                    advance = true;
                                } else if let Err(e) = result {
                                    if c.ignore_all_errors {
                                        advance = true;
                                    } else {
                                        app.copy_error_dialog = Some(app_state::CopyErrorState {
                                            operation: c.operation,
                                            message: format!("{} -> {}: {}", c.params.source_dir, name, e),
                                        });
                                        app.copy_error_focus = 0;
                                    }
                                }
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
                }
                app.copy_overwrite_dialog = None;
                if let Some(ref c) = app.copy_in_progress {
                    if c.current_index >= c.params.items.len() {
                        let source_dir = c.params.source_dir.clone();
                        let restore_after = c.params.restore_selection_after.clone();
                        let restore_before = c.params.restore_selection_before.clone();
                        app.copy_in_progress = None;
                        app.copy_progress = None;
                        app.delete_pending_rx = None;
                        restore_source_panel_and_refresh(
                            &mut app,
                            &source_dir,
                            restore_after.as_deref(),
                            restore_before.as_deref(),
                        );
                    }
                }
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
                        let _ = app.left_panel_mut().refresh_files_restore_selection(None, None, None);
                        let _ = app.right_panel_mut().refresh_files_restore_selection(None, None, None);
                    }
                }
            }
            AppAction::CopyCancel => {
                app.copy_in_progress = None;
                app.copy_progress = None;
                app.copy_overwrite_dialog = None;
                app.copy_error_dialog = None;
                app.delete_pending_rx = None;
                let _ = app.left_panel_mut().refresh_files_restore_selection(None, None, None);
                let _ = app.right_panel_mut().refresh_files_restore_selection(None, None, None);
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
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
            }
            AppAction::OpenEditor => {
                open_editor(&mut app);
            }
            AppAction::EditorSave => save(&mut app),
            AppAction::EditorClose => close(&mut app),
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
            AppAction::OpenRenameAttrDialog => rename_attr::open(&mut app),
            AppAction::OpenSizeInfoDialog => size_info_dialog::open(&mut app),
            AppAction::SizeInfoClose => size_info_dialog::close(&mut app),
            AppAction::OpenSettingsDialog => settings_dialog::open(&mut app),
            AppAction::SettingsClose => settings_dialog::close(&mut app),
            AppAction::PanelNavigated => app.maybe_persist_panel_dirs(),
            AppAction::SettingChange(change) => {
                match change {
                    SettingChange::AutosaveToggle => {
                        app.persisted_settings.autosave = !app.persisted_settings.autosave;
                    }
                    SettingChange::LeftViewCycle => {
                        let v = &mut app.persisted_settings.left_view;
                        *v = if v.as_str() == "one" {
                            "two".to_string()
                        } else {
                            "one".to_string()
                        };
                    }
                    SettingChange::LeftShowHiddenToggle => {
                        app.persisted_settings.left_show_hidden =
                            !app.persisted_settings.left_show_hidden;
                    }
                    SettingChange::RightViewCycle => {
                        let v = &mut app.persisted_settings.right_view;
                        *v = if v.as_str() == "one" {
                            "two".to_string()
                        } else {
                            "one".to_string()
                        };
                    }
                    SettingChange::RightShowHiddenToggle => {
                        app.persisted_settings.right_show_hidden =
                            !app.persisted_settings.right_show_hidden;
                    }
                    SettingChange::LeftSortCycle => {
                        let idx = crate::file_ops::SORT_MODES
                            .iter()
                            .position(|s| *s == app.persisted_settings.left_sort)
                            .unwrap_or(0);
                        let next = (idx + 1) % crate::file_ops::SORT_MODES.len();
                        app.persisted_settings.left_sort = crate::file_ops::SORT_MODES[next].to_string();
                    }
                    SettingChange::RightSortCycle => {
                        let idx = crate::file_ops::SORT_MODES
                            .iter()
                            .position(|s| *s == app.persisted_settings.right_sort)
                            .unwrap_or(0);
                        let next = (idx + 1) % crate::file_ops::SORT_MODES.len();
                        app.persisted_settings.right_sort = crate::file_ops::SORT_MODES[next].to_string();
                    }
                    SettingChange::LeftSortCyclePrev => {
                        let idx = crate::file_ops::SORT_MODES
                            .iter()
                            .position(|s| *s == app.persisted_settings.left_sort)
                            .unwrap_or(0);
                        let len = crate::file_ops::SORT_MODES.len();
                        let prev = (idx + len - 1) % len;
                        app.persisted_settings.left_sort = crate::file_ops::SORT_MODES[prev].to_string();
                    }
                    SettingChange::RightSortCyclePrev => {
                        let idx = crate::file_ops::SORT_MODES
                            .iter()
                            .position(|s| *s == app.persisted_settings.right_sort)
                            .unwrap_or(0);
                        let len = crate::file_ops::SORT_MODES.len();
                        let prev = (idx + len - 1) % len;
                        app.persisted_settings.right_sort = crate::file_ops::SORT_MODES[prev].to_string();
                    }
                    SettingChange::LeftDirsFirstToggle => {
                        app.persisted_settings.left_dirs_first = !app.persisted_settings.left_dirs_first;
                    }
                    SettingChange::RightDirsFirstToggle => {
                        app.persisted_settings.right_dirs_first = !app.persisted_settings.right_dirs_first;
                    }
                }
                let _ = settings::save(&app.persisted_settings);
                match change {
                    SettingChange::AutosaveToggle => {}
                    _ => app.sync_from_persisted_settings(),
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
                let _ = settings::save(&app.persisted_settings);
            }
            AppAction::ToggleShowHidden => {
                // Toggle only the active panel's show_hidden (left and right are independent).
                let panel_height = util::compute_panel_height();
                if app.active_panel() == 0 {
                    let new_show = !app.left_panel().get_show_hidden();
                    app.left_panel_mut().set_show_hidden(new_show);
                    app.persisted_settings.left_show_hidden = new_show;
                    app.show_hidden_files = new_show;
                    let left_name = app.left_panel().get_selected_file().map(|f| f.name.clone());
                    let _ = settings::save(&app.persisted_settings);
                    let _ = app.left_panel_mut().refresh_files_restore_selection(
                        left_name.as_deref(),
                        None,
                        Some(panel_height),
                    );
                } else {
                    let new_show = !app.right_panel().get_show_hidden();
                    app.right_panel_mut().set_show_hidden(new_show);
                    app.persisted_settings.right_show_hidden = new_show;
                    app.show_hidden_files = new_show;
                    let right_name = app.right_panel().get_selected_file().map(|f| f.name.clone());
                    let _ = settings::save(&app.persisted_settings);
                    let _ = app.right_panel_mut().refresh_files_restore_selection(
                        right_name.as_deref(),
                        None,
                        Some(panel_height),
                    );
                }
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
                if let Ok(sub) = get_or_create_subshell(&mut subshell, app.get_current_dir()) {
                    let cwd = app.get_current_dir().to_string();
                    let _ = sub.run_cd_then_relay(&cwd, Some(prepared));
                } else {
                    eprintln!("Subshell error");
                }
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
                let left_selected = app.left_panel_mut().get_selected_file().map(|f| f.name.to_string());
                let right_selected = app.right_panel_mut().get_selected_file().map(|f| f.name.to_string());
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
                if let Ok(sub) = get_or_create_subshell(&mut subshell, app.get_current_dir()) {
                    let _ = sub.run_command_then_relay(&cwd, &cmd, Some(prepared));
                } else {
                    eprintln!("Subshell error");
                }
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
                terminal.clear()?;
                let _ = app.left_panel_mut().refresh_files_restore_selection(left_selected.as_deref(), None, None);
                let _ = app.right_panel_mut().refresh_files_restore_selection(right_selected.as_deref(), None, None);
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
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
