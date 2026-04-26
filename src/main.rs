use crossterm::{
    cursor::Hide,
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

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
    refresh_both_panels_full, refresh_both_panels_restore_selection,
    restore_source_panel_and_refresh,
};
use app::settings_apply::{
    apply_persisted_setting_change, setting_change_skips_panel_resync,
    toggle_show_hidden_on_active_panel,
};
use app::state::{
    AppState, ArchiveMessage, FindDialogPhase, Focus, Operation, PostCommandCountdown,
    RenameAttrDialogState, RenameAttrField, SizeInfoDialogState, SizeInfoProgress,
};
use app::subshell_helpers::{
    get_or_create_subshell, maybe_sync_panel_to_shell_cwd, sync_subshell_root_ui_flag,
};
use browser::diff_viewer::{
    close_diff_viewer, marked_non_dir_file_count, poll_diff_loading, poll_folder_compare_pending,
    start_compare_panel_directories, try_open_diff,
};
use browser::editor::{
    apply_confirm_choice, close, finish_editor_pending_decode, open_editor, open_editor_path,
    poll_editor_loading, save, EditorViewState,
};
use browser::panel::{PanelOperations, ViewMode};
use browser::viewer::{close_viewer, open_viewer, open_viewer_path, poll_viewer_loading};
use core::location::PanelLocation;
use core::settings::{ensure_config_dir, load, save as save_settings};
use dialogs::error_detail_dialog;
use dialogs::find_dialog::FindDisplayRow;
use dialogs::pattern_select_dialog::PatternSelectMode;
use dialogs::rename_attr;
use dialogs::{
    archive_dialog, find_dialog, help_dialog, mkdir_dialog, new_file_dialog, panel_overlay,
    pattern_select_dialog, settings_dialog, size_info_dialog,
};
use shell::subshell::{RelayExit, Subshell};
use ui::post_command_overlay;
use ui::Renderer;
use util::{log_if_err, reset_terminal_character_set_and_modes};

/// When **autosave is on** and a panel path is missing in `settings.json`, use the process working
/// directory (launch directory). Falls back to `home` if `current_dir` is unavailable.
fn initial_panel_cwd_when_autosave(
    saved: Option<String>,
    home: &str,
) -> String {
    saved.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| home.to_string())
    })
}

/// Path for the **inactive** panel when autosave is off: use the saved path from disk, or `$HOME`
/// if none was stored.
fn opposite_panel_path_from_settings(
    saved: Option<String>,
    home: &str,
) -> String {
    saved.unwrap_or_else(|| home.to_string())
}

/// Restore the shell after the TUI: clear the alternate buffer when we are still on it, then leave
/// alternate screen, release raw mode, reset SGR/wrap, show the cursor, and flush so nothing lingers
/// in the emulator’s compositor.
fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    clear_alternate_buffer: bool,
) -> io::Result<()> {
    if clear_alternate_buffer {
        terminal.clear()?;
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    let _ = reset_terminal_character_set_and_modes(terminal.backend_mut());
    terminal.show_cursor()?;
    io::stdout().flush()?;
    Ok(())
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

    let app_settings = load();
    let _ = ensure_config_dir();
    log_if_err(
        "Save settings (startup)",
        save_settings(&app_settings),
    );
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let home_str = home.to_string_lossy().to_string();
    let launch_cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| home_str.clone());

    // Autosave on: restore both saved paths (missing → launch dir, not HOME).
    // Autosave off: **active** panel (`active_panel` in settings) starts in the launch directory;
    // the **opposite** panel uses the saved path for that side (`left_cwd` / `right_cwd`), or HOME
    // if nothing was stored.
    let active_is_left = app_settings.active_panel == 0;
    let (left_cwd, right_cwd) = if app_settings.autosave {
        (
            initial_panel_cwd_when_autosave(app_settings.left_cwd.clone(), &home_str),
            initial_panel_cwd_when_autosave(app_settings.right_cwd.clone(), &home_str),
        )
    } else if active_is_left {
        (
            launch_cwd.clone(),
            opposite_panel_path_from_settings(app_settings.right_cwd.clone(), &home_str),
        )
    } else {
        (
            opposite_panel_path_from_settings(app_settings.left_cwd.clone(), &home_str),
            launch_cwd.clone(),
        )
    };
    let mut app = AppState::new_with_initial(
        left_cwd,
        right_cwd,
        &home_str,
        app_settings,
    )?;
    app.sync_process_cwd_to_active_panel_if_no_autosave();
    let mut subshell: Option<Subshell> = None;

    // Process any events from EnterAlternateScreen (never discard keys).
    if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
        restore_terminal(&mut terminal, true)?;
        return Ok(());
    }
    // Show first frame; brief delay then process queue so first keypress is handled, not discarded.
    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
    std::thread::sleep(std::time::Duration::from_millis(50));
    if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
        restore_terminal(&mut terminal, true)?;
        return Ok(());
    }

    // Preload subshell so first Ctrl+O has no spawn delay (MC inits subshell at startup).
    if get_or_create_subshell(&mut subshell, app.get_current_dir()).is_ok() {
        if let Some(s) = subshell.as_ref() {
            sync_subshell_root_ui_flag(&mut app, Some(s));
        }
    } else {
        sync_subshell_root_ui_flag(&mut app, None);
    }

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
                    Hide,
                    EnableMouseCapture
                )?;
                terminal.clear()?;
                if let Some(s) = subshell.as_ref() {
                    sync_subshell_root_ui_flag(&mut app, Some(s));
                } else {
                    sync_subshell_root_ui_flag(&mut app, None);
                }
                refresh_both_panels_restore_selection(&mut app, None, None);
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

        // Editor: recv file bytes from worker before input so Esc can clear BytesLoaded before Editor::new.
        if poll_editor_loading(&mut app) {
            terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
        }

        let find_input_focused = app.find_dialog.as_ref().map_or(false, |d| {
            d.phase == FindDialogPhase::Parameter && d.focus <= 3
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
        // ratatui hides the caret when the frame sets no cursor position; `main` then called
        // `show_cursor` whenever the editor was open, which left the hardware caret at stale
        // coordinates after mouse-wheel scroll (viewport moved, caret not redrawn).
        let embedded_editor_caret_drawn = match app.editor_screen.as_ref() {
            Some(EditorViewState::Ready(ed)) => {
                if ed.search_query.is_some() {
                    true
                } else {
                    ed.editor.get_visible_cursor(&ed.area).is_some()
                }
            }
            _ => false,
        };
        let show_cursor = !app.post_command_countdown_active()
            && (embedded_editor_caret_drawn
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

        // Blink only on the shell command line. Dialog text fields use the terminal caret at a
        // fixed cell — hiding it on a 500ms timer made the caret vanish for long stretches during
        // key repeat (arrow keys), which felt like a bug.
        if command_line_cursor_active {
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
            if command_line_cursor_active {
                if cmd_cursor_blink_visible {
                    let _ = terminal.show_cursor();
                } else {
                    let _ = execute!(terminal.backend_mut(), Hide);
                }
            } else {
                let _ = terminal.show_cursor();
            }
        } else {
            let _ = execute!(terminal.backend_mut(), Hide);
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
        if poll_folder_compare_pending(&mut app) {
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
                    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
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
                // Dismiss the modal before running copy I/O so the progress overlay is visible
                // (same frame) instead of leaving the overwrite dialog up until the work finishes.
                app.copy_overwrite_dialog = None;
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                handle_copy_overwrite_choice(&mut app, n);
            }
            AppAction::CopyErrorChoice(choice) => {
                app.copy_error_dialog = None;
                if app.copy_in_progress.is_some() {
                    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                }
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
                    app.focus = Focus::FindDialog;
                }
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
            }
            AppAction::OpenDiffViewer => {
                if try_open_diff(&mut app) {
                    // Two-file full-screen diff opened.
                } else {
                    let n = marked_non_dir_file_count(&app);
                    if n == 0 {
                        start_compare_panel_directories(&mut app);
                        app.set_timed_toast(
                            std::time::Duration::from_secs(4),
                            "Panel diff: C same size · different content, S size, X only here — Esc cancels while comparing; chdir either panel clears.",
                        );
                    } else if n == 1 {
                        app.set_timed_toast(
                            std::time::Duration::from_secs(3),
                            "Mark one more file, then Ctrl+D (two files required).",
                        );
                    } else {
                        app.set_timed_toast(
                            std::time::Duration::from_secs(3),
                            "Mark exactly two files for diff; unmark the rest (Ctrl+D).",
                        );
                    }
                }
            }
            AppAction::DiffViewerClose => {
                close_diff_viewer(&mut app);
            }
            AppAction::OpenEditor => {
                open_editor(&mut app);
            }
            AppAction::EditorSave => save(&mut app),
            AppAction::EditorClose => {
                close(&mut app);
                if app.find_dialog.is_some() {
                    app.focus = Focus::FindDialog;
                }
            }
            AppAction::EditorConfirmChoice(choice) => apply_confirm_choice(&mut app, choice),
            AppAction::OpenMkdirDialog => mkdir_dialog::open(&mut app),
            AppAction::MkdirConfirm(name) => {
                mkdir_dialog::cancel(&mut app);
                let name = name.trim();
                if !name.is_empty() {
                    mkdir_dialog::create_and_refresh(&mut app, name);
                }
            }
            AppAction::MkdirCancel => mkdir_dialog::cancel(&mut app),
            AppAction::OpenPatternSelectMark => pattern_select_dialog::open_mark(&mut app),
            AppAction::OpenPatternSelectUnmark => pattern_select_dialog::open_unmark(&mut app),
            AppAction::PatternSelectConfirm => {
                if let Some(d) = pattern_select_dialog::take(&mut app) {
                    app.set_last_file_name_pattern(&d.pattern_input.text);
                    let p = d.pattern_input.text.trim();
                    if !p.is_empty() {
                        let panel_height = util::compute_panel_height();
                        let saved_index = app.active_panel_ref().get_selected_index();
                        let use_regex = app.persisted_settings.file_pattern_uses_regex();
                        let panel = app.active_panel_mut();
                        match d.mode {
                            PatternSelectMode::Mark => {
                                panel.mark_matching_glob(p, d.file_case_sensitive, use_regex);
                            }
                            PatternSelectMode::Unmark => {
                                panel.unmark_matching_glob(p, d.file_case_sensitive, use_regex);
                            }
                        }
                        panel.restore_cursor_after_same_dir_op(saved_index, panel_height);
                    }
                }
            }
            AppAction::PatternSelectCancel => pattern_select_dialog::cancel(&mut app),
            AppAction::OpenArchiveDialog => archive_dialog::open(&mut app),
            AppAction::ArchiveConfirm(name) => {
                archive_dialog::cancel(&mut app);
                let (items, ..) = app
                    .active_panel_ref()
                    .get_names_to_copy_with_restore_neighbors();
                let name = name.trim();
                if !name.is_empty() && !items.is_empty() {
                    archive_dialog::start_archive_background(&mut app, name, &items);
                }
            }
            AppAction::ArchiveCancel => archive_dialog::cancel(&mut app),
            AppAction::OpenNewFileDialog => new_file_dialog::open(&mut app),
            AppAction::NewFileConfirm(name) => {
                new_file_dialog::cancel(&mut app);
                let name = name.trim();
                if !name.is_empty() {
                    new_file_dialog::create_and_refresh(&mut app, name);
                }
            }
            AppAction::NewFileCancel => new_file_dialog::cancel(&mut app),
            AppAction::OpenRenameAttrDialog => rename_attr::open(&mut app),
            AppAction::OpenSizeInfoDialog => size_info_dialog::open(&mut app),
            AppAction::SizeInfoClose => size_info_dialog::close(&mut app),
            AppAction::OpenHelpDialog => help_dialog::open(&mut app),
            AppAction::HelpClose => help_dialog::close(&mut app),
            AppAction::ErrorDetailClose => error_detail_dialog::close(&mut app),
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
                                let loc = PanelLocation::fs(path);
                                if let Err(e) = app.active_panel_mut().navigate_to_location(loc) {
                                    error_detail_dialog::open_from_io(
                                        &mut app,
                                        "Could not open folder",
                                        e,
                                    );
                                }
                            }
                            FindDisplayRow::File(r) => {
                                if let (Some(parent), Some(name)) = (
                                    r.path.parent(),
                                    r.path.file_name().map(|n| n.to_string_lossy().to_string()),
                                ) {
                                    let loc = PanelLocation::fs(parent);
                                    let panel_height = util::compute_panel_height();
                                    match app.active_panel_mut().navigate_to_location(loc) {
                                        Ok(()) => {
                                            log_if_err(
                                                "Refresh panel",
                                                app.active_panel_mut().refresh_files_restore_selection(
                                                    Some(name.as_str()),
                                                    None,
                                                    Some(panel_height),
                                                ),
                                            );
                                        }
                                        Err(e) => {
                                            error_detail_dialog::open_from_io(
                                                &mut app,
                                                "Could not open folder",
                                                e,
                                            );
                                        }
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
                    save_settings(&app.persisted_settings),
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
                    save_settings(&app.persisted_settings),
                );
            }
            AppAction::ToggleShowHidden => {
                toggle_show_hidden_on_active_panel(&mut app);
            }
            AppAction::PersistPanelState => match app.persist_panel_state_to_settings() {
                Ok(()) => {
                    app.set_timed_toast(
                        Duration::from_secs(3),
                        "Panel layout saved to settings.",
                    );
                }
                Err(e) => {
                    app.set_timed_toast_alert(
                        Duration::from_secs(5),
                        format!("Could not save settings: {}", e),
                    );
                }
            },
            AppAction::RenameAttrConfirm => {
                if rename_attr::apply(&mut app) {
                    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                }
            }
            AppAction::RenameAttrCancel => rename_attr::cancel(&mut app),
            AppAction::Suspend => {
                // --- Ctrl+O (or Ctrl+X then O): hand terminal to subshell. Use single-writer flow (REFERENCE.md "Solution: Second Ctrl+O uglification"): do NOT use backend for leave alternate; write everything to stdout.
                let left_selected = app
                    .left_panel_mut()
                    .get_selected_file()
                    .map(|f| f.name.to_string());
                let right_selected = app
                    .right_panel_mut()
                    .get_selected_file()
                    .map(|f| f.name.to_string());
                terminal.flush()?;
                let _ = terminal.backend_mut().flush();
                let _ = std::io::stdout().flush();
                // Clear the alternate buffer while still on it so the emulator does not composite stale panel cells after 1049l.
                terminal.clear()?;
                let _ = terminal.flush();
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
                        let out = sub.get_cwd();
                        sync_subshell_root_ui_flag(&mut app, Some(sub));
                        out
                    } else {
                        sync_subshell_root_ui_flag(&mut app, None);
                        eprintln!("Subshell error");
                        None
                    };
                let _ = reset_terminal_character_set_and_modes(terminal.backend_mut());
                // 3) Return: enter alternate first (Ratatui), then drain input (MC tty_flush_input), then clear + redraw.
                execute!(
                    terminal.backend_mut(),
                    EnterAlternateScreen,
                    Hide,
                    EnableMouseCapture
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
                terminal.clear()?;
                let _ = terminal.flush();
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
                        let cwd_out = sub.get_cwd();
                        sync_subshell_root_ui_flag(&mut app, Some(sub));
                        (cwd_out, relay_exit)
                    } else {
                        sync_subshell_root_ui_flag(&mut app, None);
                        eprintln!("Subshell error");
                        (None, RelayExit::Manual)
                    };
                let _ = reset_terminal_character_set_and_modes(terminal.backend_mut());
                match relay_exit {
                    RelayExit::Manual => {
                        execute!(
                            terminal.backend_mut(),
                            EnterAlternateScreen,
                            Hide,
                            EnableMouseCapture
                        )?;
                        if let Some(AppAction::Quit) =
                            EventHandler::process_queued_events(&mut app)?
                        {
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
                        if let Some(AppAction::Quit) =
                            EventHandler::process_queued_events(&mut app)?
                        {
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

        // After input: apply diff worker result so Esc can close loading before we recv and block on work.
        if poll_diff_loading(&mut app) {
            terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
        }

        if finish_editor_pending_decode(&mut app) {
            terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
        }
    }

    restore_terminal(
        &mut terminal,
        !app.post_command_countdown_on_main_buffer(),
    )?;

    Ok(())
}
