use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::atomic::Ordering;

mod app;
mod browser;
mod core;
mod dialogs;
mod shell;
mod ui;
mod util;

use app::copy_runner::{handle_copy_overwrite_choice, run_copy_step, start_copy_operation};
use app::events::{AppAction, CopyErrorChoice, DeleteConfirmChoice, EventHandler};
use app::panel_refresh::{
    refresh_both_panels_full, refresh_both_panels_restore_selection, restore_source_panel_and_refresh,
};
use app::settings_apply::{
    apply_persisted_setting_change, setting_change_skips_panel_resync, toggle_show_hidden_on_active_panel,
};
use app::state::{
    AppState, ArchiveMessage, Focus, Operation, PostCommandCountdown, RenameAttrDialogState,
    RenameAttrField, SizeInfoDialogState, SizeInfoProgress,
};
use app::subshell_helpers::{get_or_create_subshell, maybe_sync_panel_to_shell_cwd};
use browser::editor::{apply_confirm_choice, close, open_editor, open_editor_path, save};
use browser::panel::{PanelOperations, ViewMode};
use browser::viewer::{close_viewer, open_viewer, open_viewer_path, poll_viewer_loading};
use dialogs::find_dialog::FindDisplayRow;
use dialogs::{
    archive_dialog, find_dialog, help_dialog, mkdir_dialog, new_file_dialog, panel_overlay,
    pattern_select_dialog, settings_dialog, size_info_dialog,
};
use dialogs::pattern_select_dialog::PatternSelectMode;
use dialogs::rename_attr;
use shell::subshell::{RelayExit, Subshell};
use ui::post_command_overlay;
use ui::Renderer;
use util::{log_if_err, reset_terminal_character_set_and_modes};

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
    let mut subshell: Option<Subshell> = None;

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
        // Auto-reopen countdown finished: leave main buffer (shell output) and restore panel TUI.
        let mut drew_this_frame = false;
        if let Some(cd) = app.post_command_countdown.as_ref() {
            if cd.overlay_on_main_buffer && std::time::Instant::now() >= cd.reveal_at {
                app.post_command_countdown = None;
                execute!(
                    terminal.backend_mut(),
                    EnterAlternateScreen,
                    crossterm::cursor::Hide,
                    crossterm::event::EnableMouseCapture
                )?;
                terminal.clear()?;
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                drew_this_frame = true;
            }
        }

        if !drew_this_frame {
            if app.post_command_countdown_on_main_buffer() {
                post_command_overlay::paint_main_buffer_countdown(&app)?;
            } else {
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
            }
        }

        let find_input_focused = app.find_dialog.as_ref().map_or(false, |d| {
            use crate::app::state::FindDialogPhase;
            d.phase == FindDialogPhase::Parameter && d.focus <= 2
        });
        let mkdir_input_focused = app.mkdir_dialog.as_ref().map_or(false, |d| d.focus == 0);
        let pattern_select_input_focused = app
            .pattern_select_dialog
            .as_ref()
            .map_or(false, |d| d.focus == 0);
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
            || pattern_select_input_focused
            || archive_input_focused
            || new_file_input_focused
            || rename_name_focused;
        let show_cursor = !app.post_command_countdown_active()
            && (app.editor_screen.is_some()
                || mkdir_input_focused
                || pattern_select_input_focused
                || archive_input_focused
                || new_file_input_focused
                || rename_name_focused
                || (app.focus == Focus::CommandLine)
                || find_input_focused);
        let command_line_cursor_active = app.focus == Focus::CommandLine
            && app.editor_screen.is_none()
            && !app.post_command_countdown_active()
            && app.mkdir_dialog.is_none()
            && app.pattern_select_dialog.is_none()
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
                    app.focus = crate::app::state::Focus::FindDialog;
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
                    app.focus = crate::app::state::Focus::FindDialog;
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
            AppAction::OpenPatternSelectMark => pattern_select_dialog::open_mark(&mut app),
            AppAction::OpenPatternSelectUnmark => pattern_select_dialog::open_unmark(&mut app),
            AppAction::PatternSelectConfirm => {
                if let Some(d) = pattern_select_dialog::take(&mut app) {
                    let p = d.pattern_input.text.trim();
                    if !p.is_empty() {
                        let panel_height = util::compute_panel_height();
                        let saved_index = app.active_panel_ref().get_selected_index();
                        let panel = app.active_panel_mut();
                        match d.mode {
                            PatternSelectMode::Mark => {
                                panel.mark_matching_glob(p, d.file_case_sensitive);
                            }
                            PatternSelectMode::Unmark => {
                                panel.unmark_matching_glob(p, d.file_case_sensitive);
                            }
                        }
                        panel.restore_cursor_after_same_dir_op(saved_index, panel_height);
                    }
                }
            }
            AppAction::PatternSelectCancel => pattern_select_dialog::cancel(&mut app),
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
                            FindDisplayRow::Folder(path) => {
                                let loc = crate::core::location::PanelLocation::fs(path);
                                if app.active_panel_mut().navigate_to_location(loc).is_ok() {}
                            }
                            FindDisplayRow::File(r) => {
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
                    if let Some(FindDisplayRow::File(r)) = rows.get(d.selected_index) {
                        if r.path.is_file() {
                            open_viewer_path(&mut app, r.path.clone(), r.line);
                        }
                    }
                }
                // Do not close Find dialog: user returns to it after closing viewer.
            }
            AppAction::FindEdit => {
                if let Some(ref d) = app.find_dialog {
                    let rows = find_dialog::build_display_rows(&d.results);
                    if let Some(FindDisplayRow::File(r)) = rows.get(d.selected_index) {
                        if r.path.is_file() {
                            open_editor_path(&mut app, r.path.clone());
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
                let prepared = Subshell::prepare_for_relay();
                {
                    let mut stdout = std::io::stdout().lock();
                    let _ = Subshell::write_relay_reset_sequence(&mut stdout);
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
                let prepared = Subshell::prepare_for_relay();
                {
                    let mut stdout = std::io::stdout().lock();
                    let _ = Subshell::write_relay_reset_sequence(&mut stdout);
                    let _ = stdout.flush();
                }
                let (shell_cwd, relay_exit) =
                    if let Ok(sub) = get_or_create_subshell(&mut subshell, app.get_current_dir()) {
                        let relay_exit = sub.run_command_then_relay(
                            &cwd,
                            &cmd,
                            Some(prepared),
                            auto_exit_after_idle,
                        )?;
                        (sub.get_cwd(), relay_exit)
                    } else {
                        eprintln!("Subshell error");
                        (None, RelayExit::Manual)
                    };
                let _ = reset_terminal_character_set_and_modes(terminal.backend_mut());
                match relay_exit {
                    RelayExit::Manual => {
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
                    }
                    RelayExit::AutoReopenDelay(d) => {
                        if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                            break;
                        }
                        app.focus_panel();
                        maybe_sync_panel_to_shell_cwd(&mut app, shell_cwd);
                        refresh_both_panels_restore_selection(
                            &mut app,
                            left_selected.as_deref(),
                            right_selected.as_deref(),
                        );
                        app.post_command_countdown = Some(PostCommandCountdown {
                            reveal_at: std::time::Instant::now() + d,
                            overlay_on_main_buffer: true,
                        });
                        post_command_overlay::paint_main_buffer_countdown(&app)?;
                    }
                }
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
