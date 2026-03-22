use crate::app::state::{AppState, CopyParams, Focus, RenameAttrDialogState, RenameAttrField};
pub use crate::browser::editor::EditorConfirmChoice;
use crate::browser::editor::{handle_editor_key, handle_editor_mouse};
use crate::browser::panel::PanelOperations;
use crate::ui::Renderer;
use crate::browser::viewer::handle_viewer_key;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    terminal::size,
};
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    Continue,
    Quit,
    Suspend,
    RunCommand(String),
    /// F5 Copy: copy selected/marked files from active panel to opposite panel.
    Copy(CopyParams),
    /// F6 Move: move selected/marked files from active panel to opposite panel.
    Move(CopyParams),
    /// User chose 1–4 in the "file exists" overwrite dialog during copy.
    CopyOverwriteChoice(usize),
    /// User chose Ignore or Cancel in the copy error dialog.
    CopyErrorChoice(CopyErrorChoice),
    /// User pressed ESC to cancel the copy operation.
    CopyCancel,
    /// User chose Yes or No in the "Are you sure?" delete confirmation dialog.
    DeleteConfirmChoice(DeleteConfirmChoice),
    /// F4: open current file in embedded editor.
    OpenEditor,
    /// F2 in editor: save file.
    EditorSave,
    /// ESC in editor with no unsaved changes: close editor.
    EditorClose,
    /// User chose Save / Discard / Cancel in "Save changes?" dialog.
    EditorConfirmChoice(EditorConfirmChoice),
    /// F3 in panel: open current file in viewer.
    OpenViewer,
    /// ESC in viewer: close viewer.
    ViewerClose,
    /// F7: open "Create a new Directory" dialog (MC-style).
    OpenMkdirDialog,
    /// Enter in mkdir dialog: create directory and close.
    MkdirConfirm,
    /// ESC in mkdir dialog: cancel and close.
    MkdirCancel,
    /// Ctrl+A: open "Archive" dialog (create zip of selected items; originals kept).
    OpenArchiveDialog,
    /// Enter in archive dialog: create archive and close.
    ArchiveConfirm,
    /// ESC in archive dialog: cancel and close.
    ArchiveCancel,
    /// ESC during archive progress: stop archiving and close progress dialog (like CopyCancel).
    ArchiveProgressCancel,
    /// Ctrl+N: open "New file" dialog (create empty file in current directory or archive).
    OpenNewFileDialog,
    /// Enter in new file dialog: create file and close (or show error if exists).
    NewFileConfirm,
    /// ESC in new file dialog: cancel and close.
    NewFileCancel,
    /// F2: open "Rename / Attributes" dialog (single file or group).
    OpenRenameAttrDialog,
    /// Enter in F2 dialog: apply rename + chmod and close.
    RenameAttrConfirm,
    /// ESC in F2 dialog: cancel and close.
    RenameAttrCancel,
    /// Ctrl+G: open "Size info" dialog (total size of selected files and folders).
    OpenSizeInfoDialog,
    /// ESC in size info dialog: close.
    SizeInfoClose,
    /// F1: open Help dialog.
    OpenHelpDialog,
    /// ESC or mouse click in Help dialog: close.
    HelpClose,
    /// F9: open Settings dialog.
    OpenSettingsDialog,
    /// ESC or mouse click in Settings dialog: close.
    SettingsClose,
    /// Ctrl+Q: open Left panel settings overlay over the left panel.
    OpenLeftPanelSettings,
    /// Ctrl+W: open Right panel settings overlay over the right panel.
    OpenRightPanelSettings,
    /// Close Left panel settings overlay.
    CloseLeftPanelSettings,
    /// Close Right panel settings overlay.
    CloseRightPanelSettings,
    /// Ctrl+F: open Find file dialog.
    OpenFindDialog,
    /// Close Find file dialog (ESC / Cancel).
    FindClose,
    /// In Find results: Chdir to selected result's directory.
    FindChdir,
    /// In Find results: View selected file (F3). Find dialog stays open.
    FindView,
    /// In Find results: Edit selected file (F4). Find dialog stays open.
    FindEdit,
    /// Start Find file search (from Find button in parameter form).
    FindStartSearch,
    /// +: open dialog to mark files matching a glob (same as Find file pattern).
    OpenPatternSelectMark,
    /// −: open dialog to unmark files matching a glob.
    OpenPatternSelectUnmark,
    /// Apply pattern selection (mark or unmark).
    PatternSelectConfirm,
    /// Close +/− pattern dialog without applying.
    PatternSelectCancel,
    /// Ctrl+H: toggle hidden files visibility.
    ToggleShowHidden,
    /// Panel directory changed (Enter or double-click on dir). Used for autosave of panel cwds.
    PanelNavigated,
    /// A specific setting was toggled/changed in the F9 Settings dialog. Main applies to persisted_settings, saves, applies to panels.
    SettingChange(SettingChange),
    /// Ctrl+T toggled view mode; persist to file.
    ViewModeToggled,
}

/// Single source of truth: each variant updates one field in PersistedSettings. Applied in main.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingChange {
    AutosaveToggle,
    /// Sync active panel to shell's cwd when returning from Ctrl+O (F9 Settings → General).
    SyncPanelToShellCwdToggle,
    /// Toggle auto-return to panels after running a command/executable.
    AutoReopenPanelsAfterCommandToggle,
    /// Cycle the delay (seconds) for auto-return to panels.
    AutoReopenPanelsAfterCommandDelayCycle,
    LeftViewCycle,
    LeftShowHiddenToggle,
    LeftSortCycle,
    LeftSortCyclePrev,
    LeftDirsFirstToggle,
    RightViewCycle,
    RightShowHiddenToggle,
    RightSortCycle,
    RightSortCyclePrev,
    RightDirsFirstToggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteConfirmChoice {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyErrorChoice {
    Ignore,
    Cancel,
    /// Ignore this file and all future errors (skip without showing dialog).
    IgnoreAll,
}

pub struct EventHandler;

impl EventHandler {
    /// When false, mouse must not scroll panels, click the file list, or use the bottom menu bar
    /// (dialogs and progress overlays are modal).
    fn panels_and_menu_mouse_enabled(app: &AppState) -> bool {
        app.viewer_screen.is_none()
            && app.editor_screen.is_none()
            && !app.editor_confirm_pending
            && app.copy_overwrite_dialog.is_none()
            && app.copy_error_dialog.is_none()
            && app.operation_confirm_pending.is_none()
            && app.mkdir_dialog.is_none()
            && app.pattern_select_dialog.is_none()
            && app.archive_dialog.is_none()
            && app.new_file_dialog.is_none()
            && app.new_file_error.is_none()
            && app.rename_attr_dialog.is_none()
            && app.size_info_dialog.is_none()
            && app.settings_dialog.is_none()
            && !app.help_dialog
            && app.find_dialog.is_none()
            && app.left_panel_settings_overlay.is_none()
            && app.right_panel_settings_overlay.is_none()
            && app.copy_in_progress.is_none()
            && app.archive_progress.is_none()
    }

    /// While any of these are open, bracketed paste must not reach the command line (focus can be stale).
    fn modal_blocks_command_line_paste(app: &AppState) -> bool {
        app.copy_overwrite_dialog.is_some()
            || app.copy_error_dialog.is_some()
            || app.operation_confirm_pending.is_some()
            || app.mkdir_dialog.is_some()
            || app.pattern_select_dialog.is_some()
            || app.archive_dialog.is_some()
            || app.new_file_dialog.is_some()
            || app.new_file_error.is_some()
            || app.rename_attr_dialog.is_some()
            || app.size_info_dialog.is_some()
            || app.settings_dialog.is_some()
            || app.help_dialog
            || app.find_dialog.is_some()
            || app.left_panel_settings_overlay.is_some()
            || app.right_panel_settings_overlay.is_some()
            || app.editor_confirm_pending
            || app.copy_progress.is_some()
            || app.archive_progress.is_some()
    }

    /// Drain the crossterm queue without blocking; return the last key/mouse action, if any.
    fn drain_events_nonblocking(app: &mut AppState) -> io::Result<Option<AppAction>> {
        let mut last_action = None;
        while event::poll(std::time::Duration::ZERO)? {
            let ev = event::read()?;
            if let Some(action) = Self::dispatch_event(app, ev)? {
                last_action = Some(action);
            }
        }
        Ok(last_action)
    }

    /// Process all pending events: handle Key/Mouse (update app, return action), ignore FocusGained/Resize.
    /// Never discards keys. Returns the last action from a key/mouse, or None if queue empty or only non-keys.
    pub fn process_queued_events(app: &mut AppState) -> io::Result<Option<AppAction>> {
        Self::drain_events_nonblocking(app)
    }

    /// During post-command countdown: advance time without dispatching keys/mouse (panels not visible yet).
    fn handle_events_post_command_countdown() -> io::Result<AppAction> {
        while event::poll(std::time::Duration::ZERO)? {
            let _ = event::read()?;
        }
        if !event::poll(std::time::Duration::from_millis(100))? {
            return Ok(AppAction::Continue);
        }
        let _ = event::read()?;
        Ok(AppAction::Continue)
    }

    pub fn handle_events(app: &mut AppState) -> io::Result<AppAction> {
        if app.post_command_countdown_active() {
            return Self::handle_events_post_command_countdown();
        }
        // Process all queued events first (no block). Handle every Key/Mouse; drain non-keys.
        // This ensures rapid keypresses when switching panels (e.g. Tab then Down) are all applied.
        if let Some(action) = Self::drain_events_nonblocking(app)? {
            return Ok(action);
        }
        // Queue empty: block for one event.
        if !event::poll(std::time::Duration::from_millis(100))? {
            return Ok(AppAction::Continue);
        }
        let ev = event::read()?;
        if let Some(action) = Self::dispatch_event(app, ev)? {
            return Ok(action);
        }
        Ok(AppAction::Continue)
    }

    /// Dispatch one event. Returns Some(action) if we handled a key and should return it, None to continue/drain.
    ///
    /// **Panel ↔ command line flow (MC-style):**
    /// - **Focus** is the single source of truth: `Panel` (default) or `CommandLine`.
    /// - **Panel → command line:** Type a printable character (focus moves and char is inserted), press **F6** (focus only), or **Esc** (focus only; cursor position preserved).
    /// - **Command line → panel:** **Tab** or **Esc** (focus returns to active panel; command line text and cursor position kept).
    /// - **Between panels:** **Tab** when focus is Panel switches left/right panel; from command line Tab first returns focus to panel.
    /// - All key handling branches on `app.focus` first; no key is handled by both panel and command line.
    fn dispatch_event(
        app: &mut AppState,
        ev: Event,
    ) -> io::Result<Option<AppAction>> {
        match ev {
            Event::Key(key) => {
                // When viewer is open, it gets keys first (e.g. opened from Find F3; Find stays open behind).
                if let Some(action) = handle_viewer_key(app, key) {
                    return Ok(Some(action));
                }
                // When editor "Save changes?" dialog is open: 1/2/3 direct, Tab/↑↓ cycle, Enter confirms, Esc=Cancel.
                if app.editor_confirm_pending {
                    use crate::browser::editor::EditorConfirmChoice;
                    let choice = match key.code {
                        KeyCode::Char('1') => Some(EditorConfirmChoice::Save),
                        KeyCode::Char('2') => Some(EditorConfirmChoice::Discard),
                        KeyCode::Char('3') | KeyCode::Esc => Some(EditorConfirmChoice::Cancel),
                        KeyCode::Tab | KeyCode::Char('\t') => {
                            app.editor_confirm_focus = (app.editor_confirm_focus + 1) % 3;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::BackTab => {
                            app.editor_confirm_focus = (app.editor_confirm_focus + 2) % 3;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Up => {
                            app.editor_confirm_focus = (app.editor_confirm_focus + 2) % 3;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Down => {
                            app.editor_confirm_focus = (app.editor_confirm_focus + 1) % 3;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Enter => Some(match app.editor_confirm_focus {
                            0 => EditorConfirmChoice::Save,
                            1 => EditorConfirmChoice::Discard,
                            _ => EditorConfirmChoice::Cancel,
                        }),
                        _ => None,
                    };
                    if let Some(c) = choice {
                        return Ok(Some(AppAction::EditorConfirmChoice(c)));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // When embedded editor is open, delegate to editor module.
                if let Some(action) = handle_editor_key(app, key) {
                    return Ok(Some(action));
                }
                // When Find file dialog is open it has focus (unless viewer/editor is on top). Keys go to Find, not panel.
                if app.find_dialog.is_some() {
                    let action = crate::dialogs::find_dialog::handle_key(app, key.code, key.modifiers)
                        .unwrap_or(AppAction::Continue);
                    return Ok(Some(action));
                }
                // When overwrite dialog is open: 1–5 direct, Tab/↑↓ cycle, Enter confirms, Esc=Cancel. Keys differ from Yes/No.
                if app.copy_overwrite_dialog.is_some() {
                    let choice = match key.code {
                        KeyCode::Char('1') => Some(1),
                        KeyCode::Char('2') => Some(2),
                        KeyCode::Char('3') => Some(3),
                        KeyCode::Char('4') => Some(4),
                        KeyCode::Char('5') | KeyCode::Esc => Some(5),
                        KeyCode::Tab | KeyCode::Char('\t') => {
                            app.copy_overwrite_focus = (app.copy_overwrite_focus + 1) % 5;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::BackTab => {
                            app.copy_overwrite_focus = (app.copy_overwrite_focus + 4) % 5;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Up => {
                            app.copy_overwrite_focus = (app.copy_overwrite_focus + 4) % 5;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Down => {
                            app.copy_overwrite_focus = (app.copy_overwrite_focus + 1) % 5;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Enter => {
                            return Ok(Some(AppAction::CopyOverwriteChoice(
                                app.copy_overwrite_focus + 1,
                            )));
                        }
                        _ => None,
                    };
                    if let Some(n) = choice {
                        return Ok(Some(AppAction::CopyOverwriteChoice(n)));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // When operation confirmation is pending: Y/y=Yes, N/n/Esc=No, Tab=switch, Enter=confirm.
                if app.operation_confirm_pending.is_some() {
                    let choice = match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => Some(DeleteConfirmChoice::Yes),
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            Some(DeleteConfirmChoice::No)
                        }
                        KeyCode::Tab | KeyCode::Char('\t') => {
                            app.operation_confirm_focus_yes = !app.operation_confirm_focus_yes;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Enter => Some(if app.operation_confirm_focus_yes {
                            DeleteConfirmChoice::Yes
                        } else {
                            DeleteConfirmChoice::No
                        }),
                        _ => None,
                    };
                    if let Some(confirm_choice) = choice {
                        return Ok(Some(AppAction::DeleteConfirmChoice(confirm_choice)));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // When new file error (e.g. file exists) is open: Enter or Esc closes.
                if app.new_file_error.is_some() {
                    if key.code == KeyCode::Enter || key.code == KeyCode::Esc {
                        app.new_file_error = None;
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // When copy error dialog is open: 1–3 direct, Tab/↑↓ cycle, Enter confirms, Esc=Cancel. Keys differ from Yes/No.
                if app.copy_error_dialog.is_some() {
                    let choice = match key.code {
                        KeyCode::Char('1') => Some(CopyErrorChoice::Ignore),
                        KeyCode::Char('2') | KeyCode::Esc => Some(CopyErrorChoice::Cancel),
                        KeyCode::Char('3') => Some(CopyErrorChoice::IgnoreAll),
                        KeyCode::Tab | KeyCode::Char('\t') => {
                            app.copy_error_focus = (app.copy_error_focus + 1) % 3;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::BackTab => {
                            app.copy_error_focus = (app.copy_error_focus + 2) % 3;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Up => {
                            app.copy_error_focus = (app.copy_error_focus + 2) % 3;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Down => {
                            app.copy_error_focus = (app.copy_error_focus + 1) % 3;
                            return Ok(Some(AppAction::Continue));
                        }
                        KeyCode::Enter => {
                            let error_choice = match app.copy_error_focus {
                                0 => CopyErrorChoice::Ignore,
                                1 => CopyErrorChoice::Cancel,
                                _ => CopyErrorChoice::IgnoreAll,
                            };
                            return Ok(Some(AppAction::CopyErrorChoice(error_choice)));
                        }
                        _ => None,
                    };
                    if let Some(error_choice) = choice {
                        return Ok(Some(AppAction::CopyErrorChoice(error_choice)));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // Copy/move/delete progress overlay: modal — only Esc cancels; other keys do not reach panels.
                if app.copy_in_progress.is_some()
                    && app.copy_overwrite_dialog.is_none()
                    && app.copy_error_dialog.is_none()
                {
                    if key.code == KeyCode::Esc {
                        return Ok(Some(AppAction::CopyCancel));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // When archive is in progress, Esc cancels (same as Copy/Move: stop and close dialog).
                if app.archive_progress.is_some() {
                    if key.code == KeyCode::Esc {
                        return Ok(Some(AppAction::ArchiveProgressCancel));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // F7 "Create directory" dialog: modal — same as F1: never leak keys to panel/command line.
                if app.mkdir_dialog.is_some() {
                    return Ok(Some(
                        crate::dialogs::mkdir_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                if app.pattern_select_dialog.is_some() {
                    return Ok(Some(
                        crate::dialogs::pattern_select_dialog::handle_key(
                            app,
                            key.code,
                            key.modifiers,
                        )
                        .unwrap_or(AppAction::Continue),
                    ));
                }
                // Ctrl+A "Archive" dialog
                if app.archive_dialog.is_some() {
                    return Ok(Some(
                        crate::dialogs::archive_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Ctrl+N "New file" dialog
                if app.new_file_dialog.is_some() {
                    return Ok(Some(
                        crate::dialogs::new_file_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Panel settings overlay (Ctrl+Q left, Ctrl+W right)
                if app.left_panel_settings_overlay.is_some()
                    || app.right_panel_settings_overlay.is_some()
                {
                    return Ok(Some(
                        crate::dialogs::panel_overlay::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // F1 Help dialog: Esc/q close; all other keys are absorbed (modal).
                if app.help_dialog {
                    return Ok(Some(
                        crate::dialogs::help_dialog::handle_key(key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // F9 Settings dialog
                if app.settings_dialog.is_some() {
                    return Ok(Some(
                        crate::dialogs::settings_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Size info dialog
                if app.size_info_dialog.is_some() {
                    return Ok(Some(
                        crate::dialogs::size_info_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // F2 "Rename / Attributes" dialog
                if app.rename_attr_dialog.is_some() {
                    if app.rename_attr_error.is_some() {
                        app.rename_attr_error = None;
                        return Ok(Some(AppAction::Continue));
                    }
                    return Ok(Some(
                        crate::dialogs::rename_attr::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Some terminals send Tab as Char('\t'); treat it as Tab.
                let code = match key.code {
                    KeyCode::Char('\t') => KeyCode::Tab,
                    other => other,
                };
                if app.focus == Focus::CommandLine {
                    return Ok(Some(Self::handle_command_line_key(
                        app,
                        code,
                        key.modifiers,
                    )));
                }
                // Panel height: outer frame, inner content; visible list rows = terminal - 4.
                let panel_height = crate::util::compute_panel_height();
                // Panel has focus: navigation, panel switch, or move to command line.
                match code {
                    KeyCode::Up => app.active_panel_mut().move_up(panel_height),
                    KeyCode::Down => app.active_panel_mut().move_down(panel_height),
                    KeyCode::Left => app.active_panel_mut().smart_move_left(panel_height),
                    KeyCode::Right => app.active_panel_mut().smart_move_right(panel_height),
                    KeyCode::PageUp => app.active_panel_mut().page_up(panel_height),
                    KeyCode::PageDown => app.active_panel_mut().page_down(panel_height),
                    KeyCode::Enter => {
                        let panel = app.active_panel_mut();
                        if let Some(file) = panel.get_selected_file() {
                            if !file.is_dir && file.is_executable {
                                let cmd = format!("./{}", file.name);
                                return Ok(Some(AppAction::RunCommand(cmd)));
                            }
                        }
                        panel.enter_directory()?;
                        app.sync_process_cwd_to_active_panel();
                        return Ok(Some(AppAction::PanelNavigated));
                    }
                    KeyCode::Char(' ') => {
                        // Space = toggle mark on the current file, then move selection down.
                        app.active_panel_mut()
                            .toggle_mark_and_move_next(panel_height);
                    }
                    KeyCode::Char(c) => {
                        if c == '*' {
                            app.active_panel_mut().invert_selection();
                        } else if c == '+' {
                            return Ok(Some(AppAction::OpenPatternSelectMark));
                        } else if c == '-' {
                            return Ok(Some(AppAction::OpenPatternSelectUnmark));
                        } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                            return Ok(Some(Self::handle_ctrl_key(app, c, panel_height)));
                        } else if c.is_ascii() && !c.is_control() {
                            app.focus_command_line();
                            app.command_line_insert(c);
                        }
                    }
                    KeyCode::Tab => app.switch_panel()?,
                    KeyCode::Esc => app.focus_command_line(),
                    KeyCode::F(5) => {
                        let source = app.get_current_dir().to_string();
                        let target = app.get_opposite_panel_dir().to_string();
                        if source != target {
                            let (items, restore_after, restore_before) = app
                                .active_panel_mut()
                                .get_names_to_copy_with_restore_neighbors();
                            if !items.is_empty() {
                                let opposite = app.get_opposite_panel_location();
                                let (target_location, target_fs_path) = match &opposite {
                                    crate::core::location::PanelLocation::Zip { .. } => {
                                        (Some(opposite), None)
                                    }
                                    crate::core::location::PanelLocation::Fs(_) => {
                                        (None, Some(app.get_opposite_panel_target_fs_path()))
                                    }
                                };
                                return Ok(Some(AppAction::Copy(CopyParams {
                                    source_dir: source,
                                    target_dir: target,
                                    source_location: Some(app.get_current_location()),
                                    target_location,
                                    target_fs_path,
                                    items,
                                    restore_selection_after: restore_after,
                                    restore_selection_before: restore_before,
                                })));
                            }
                        }
                    }
                    KeyCode::F(6) => {
                        let source = app.get_current_dir().to_string();
                        let target = app.get_opposite_panel_dir().to_string();
                        if source != target {
                            let (items, restore_after, restore_before) = app
                                .active_panel_mut()
                                .get_names_to_copy_with_restore_neighbors();
                            if !items.is_empty() {
                                let opposite = app.get_opposite_panel_location();
                                let (target_location, target_fs_path) = match &opposite {
                                    crate::core::location::PanelLocation::Zip { .. } => {
                                        (Some(opposite), None)
                                    }
                                    crate::core::location::PanelLocation::Fs(_) => {
                                        (None, Some(app.get_opposite_panel_target_fs_path()))
                                    }
                                };
                                return Ok(Some(AppAction::Move(CopyParams {
                                    source_dir: source,
                                    target_dir: target,
                                    source_location: Some(app.get_current_location()),
                                    target_location,
                                    target_fs_path,
                                    items,
                                    restore_selection_after: restore_after,
                                    restore_selection_before: restore_before,
                                })));
                            }
                        }
                        app.focus_command_line(); // MC: no selection or same dir = focus command line
                    }
                    KeyCode::F(3) => {
                        if let Some(file) = app.active_panel_mut().get_selected_file() {
                            if !file.is_dir && !file.is_parent_dir() {
                                return Ok(Some(AppAction::OpenViewer));
                            }
                        }
                    }
                    KeyCode::F(1) => return Ok(Some(AppAction::OpenHelpDialog)),
                    KeyCode::F(9) => return Ok(Some(AppAction::OpenSettingsDialog)),
                    KeyCode::F(7) => {
                        if crate::core::panel_backend::supports_mkdir(&app.get_current_location()) {
                            return Ok(Some(AppAction::OpenMkdirDialog));
                        }
                    }
                    KeyCode::F(2) => return Ok(Some(AppAction::OpenRenameAttrDialog)),
                    KeyCode::F(4) => {
                        if crate::core::panel_backend::supports_edit(&app.get_current_location()) {
                            if let Some(file) = app.active_panel_mut().get_selected_file() {
                                if !file.is_dir && !file.is_parent_dir() {
                                    return Ok(Some(AppAction::OpenEditor));
                                }
                            }
                        }
                    }
                    KeyCode::F(8) => {
                        let (items, restore_after, restore_before) = app
                            .active_panel_mut()
                            .get_names_to_copy_with_restore_neighbors();
                        if !items.is_empty() {
                            app.operation_confirm_pending = Some((
                                crate::app::state::Operation::Delete,
                                CopyParams {
                                    source_dir: app.get_current_dir().to_string(),
                                    target_dir: String::new(),
                                    source_location: Some(app.get_current_location()),
                                    target_location: None,
                                    target_fs_path: None,
                                    items,
                                    restore_selection_after: restore_after,
                                    restore_selection_before: restore_before,
                                },
                            ));
                            app.operation_confirm_focus_yes = true;
                        }
                    }
                    KeyCode::F(10) => return Ok(Some(AppAction::Quit)),
                    _ => {}
                }
                Ok(Some(AppAction::Continue))
            }
            Event::Mouse(mouse_event) => {
                if app.editor_confirm_pending {
                    return Ok(Some(AppAction::Continue));
                }
                if handle_editor_mouse(app, mouse_event) {
                    return Ok(Some(AppAction::Continue));
                }
                let action = Self::handle_mouse_event(app, mouse_event)?;
                Ok(Some(action.unwrap_or(AppAction::Continue)))
            }
            // Bracketed paste (e.g. Cmd+V on macOS): editor first if open, else focused dialog/command line.
            Event::Paste(data) => {
                if app.editor_screen.is_some() {
                    if let Some(action) = crate::browser::editor::paste_text_as_is(app, &data) {
                        return Ok(Some(action));
                    }
                }
                if Self::paste_into_focused_input(app, &data) {
                    return Ok(Some(AppAction::Continue));
                }
                if Self::modal_blocks_command_line_paste(app) {
                    return Ok(Some(AppAction::Continue));
                }
                Ok(None)
            }
            _ => Ok(None), // FocusGained, Resize, etc. - drain
        }
    }

    /// If a dialog text input or the command line has logical focus, insert `data` there and return true.
    fn paste_into_focused_input(
        app: &mut AppState,
        data: &str,
    ) -> bool {
        if let Some(d) = app.find_dialog.as_mut() {
            if d.focus <= 2 {
                let input = match d.focus {
                    0 => &mut d.start_dir_input,
                    1 => &mut d.file_pattern_input,
                    2 => &mut d.content_pattern_input,
                    _ => return false,
                };
                *input = std::mem::take(input).insert_str(data);
                return true;
            }
        }
        if let Some(d) = app.pattern_select_dialog.as_mut() {
            if d.focus == 0 {
                d.pattern_input = std::mem::take(&mut d.pattern_input).insert_str(data);
                return true;
            }
        }
        if let Some(d) = app.mkdir_dialog.as_mut() {
            if d.focus == 0 {
                d.input = std::mem::take(&mut d.input).insert_str(data);
                return true;
            }
        }
        if let Some(d) = app.archive_dialog.as_mut() {
            if d.focus == 0 {
                d.input = std::mem::take(&mut d.input).insert_str(data);
                return true;
            }
        }
        if let Some(d) = app.new_file_dialog.as_mut() {
            if d.focus == 0 {
                d.input = std::mem::take(&mut d.input).insert_str(data);
                return true;
            }
        }
        if let Some(RenameAttrDialogState::Single {
            name_input, focus, ..
        }) = app.rename_attr_dialog.as_mut()
        {
            if *focus == RenameAttrField::Name {
                *name_input = std::mem::take(name_input).insert_str(data);
                return true;
            }
        }
        if app.focus == Focus::CommandLine {
            if Self::modal_blocks_command_line_paste(app) {
                return true;
            }
            app.command_line_insert_str(data);
            return true;
        }
        false
    }

    fn handle_command_line_key(
        app: &mut AppState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> AppAction {
        // F10 in command prompt mode: exit immediately.
        if code == KeyCode::F(10) {
            return AppAction::Quit;
        }
        // Tab may be passed as KeyCode::Tab (normalized from Char('\t') in dispatch_event).
        match code {
            KeyCode::Char(c) => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    if c == 'q' {
                        return AppAction::OpenLeftPanelSettings;
                    }
                    if c == 'w' {
                        return AppAction::OpenRightPanelSettings;
                    }
                    if c == 'o' {
                        return AppAction::Suspend;
                    }
                    if c == 'c' {
                        if !app.command_line.is_empty() {
                            crate::browser::clipboard::set(&app.command_line);
                        } else {
                            app.command_line_clear();
                        }
                        return AppAction::Continue;
                    }
                    if c == 'h' {
                        return AppAction::ToggleShowHidden;
                    }
                    if c == 'v' {
                        if let Some(s) = crate::browser::clipboard::get() {
                            app.command_line_insert_str(&s);
                        }
                        return AppAction::Continue;
                    }
                }
                app.command_line_insert(c);
                AppAction::Continue
            }
            KeyCode::Backspace => {
                app.command_line_backspace();
                if app.command_line.is_empty() {
                    app.focus_panel();
                }
                AppAction::Continue
            }
            KeyCode::Left => {
                app.command_line_move_left();
                AppAction::Continue
            }
            KeyCode::Right => {
                app.command_line_move_right();
                AppAction::Continue
            }
            KeyCode::F(12) => {
                // F12: insert current file at cursor only (don't run). Works on all terminals and macOS
                // where Ctrl+Enter and Option+Enter are often consumed or not reported.
                let name = app
                    .active_panel_ref()
                    .get_selected_file()
                    .map(|f| f.name.clone());
                if let Some(name) = name {
                    app.command_line_insert_str(&name);
                }
                AppAction::Continue
            }
            KeyCode::Enter => {
                let cmd = app.take_command_line();
                if !cmd.trim().is_empty() {
                    AppAction::RunCommand(cmd)
                } else {
                    AppAction::Continue
                }
            }
            KeyCode::Tab => {
                app.focus_panel();
                AppAction::Continue
            }
            KeyCode::Esc => {
                app.focus_panel();
                AppAction::Continue
            }
            _ => AppAction::Continue,
        }
    }

    fn handle_ctrl_key(
        app: &mut AppState,
        c: char,
        panel_height: usize,
    ) -> AppAction {
        match c {
            'q' => AppAction::OpenLeftPanelSettings,
            'w' => AppAction::OpenRightPanelSettings,
            'o' => AppAction::Suspend,
            'g' => {
                let (items, ..) = app
                    .active_panel_ref()
                    .get_names_to_copy_with_restore_neighbors();
                if !items.is_empty() {
                    AppAction::OpenSizeInfoDialog
                } else {
                    AppAction::Continue
                }
            }
            't' => {
                app.toggle_view_mode();
                AppAction::ViewModeToggled
            }
            'f' => {
                if app.find_dialog.is_none() {
                    AppAction::OpenFindDialog
                } else {
                    AppAction::Continue
                }
            }
            'h' => AppAction::ToggleShowHidden,
            'a' => {
                let loc = app.get_current_location();
                if loc.is_fs() {
                    let (items, ..) = app
                        .active_panel_ref()
                        .get_names_to_copy_with_restore_neighbors();
                    if !items.is_empty() {
                        return AppAction::OpenArchiveDialog;
                    }
                }
                AppAction::Continue
            }
            'n' => {
                if crate::core::panel_backend::supports_new_file(&app.get_current_location()) {
                    return AppAction::OpenNewFileDialog;
                }
                AppAction::Continue
            }
            'r' => {
                let _ = app.active_panel_mut().refresh_files_restore_selection(
                    None,
                    None,
                    Some(panel_height),
                );
                AppAction::Continue
            }
            _ => AppAction::Continue,
        }
    }

    /// Returns Ok(Some(action)) when an action (e.g. RunCommand) should be handled by the main loop.
    fn handle_mouse_event(
        app: &mut AppState,
        mouse_event: MouseEvent,
    ) -> io::Result<Option<AppAction>> {
        // Store pointer position from every mouse event (scroll, move, click) so Space can "select at pointer".
        app.last_mouse_position = Some((mouse_event.column, mouse_event.row));

        let (term_w, term_h) = size().unwrap_or((80, 24));
        let area = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: term_w,
            height: term_h,
        };

        // Overwrite dialog: handle clicks on option rows (1–5). Mouse/touchpad friendly.
        if app.copy_overwrite_dialog.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                let (_rect, content) = Renderer::overwrite_dialog_layout(area);
                let (col, row) = (mouse_event.column, mouse_event.row);
                if col >= content.x && col < content.x + content.width && row >= content.y + 2 {
                    let opt_row = (row - content.y - 2) as usize;
                    if opt_row < 5 {
                        return Ok(Some(AppAction::CopyOverwriteChoice(opt_row + 1)));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // Error dialog: handle clicks on option rows (1–3).
        if app.copy_error_dialog.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                let (_rect, content) = Renderer::error_dialog_layout(area);
                let (col, row) = (mouse_event.column, mouse_event.row);
                if col >= content.x && col < content.x + content.width && row >= content.y + 2 {
                    let opt_row = (row - content.y - 2) as usize;
                    if opt_row < 3 {
                        let error_choice = match opt_row {
                            0 => CopyErrorChoice::Ignore,
                            1 => CopyErrorChoice::Cancel,
                            _ => CopyErrorChoice::IgnoreAll,
                        };
                        return Ok(Some(AppAction::CopyErrorChoice(error_choice)));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // Copy/move/delete progress (when no overwrite/error dialog on top)
        if app.copy_progress.is_some() {
            return Ok(Some(AppAction::Continue));
        }
        // Archive progress overlay
        if app.archive_progress.is_some() {
            return Ok(Some(AppAction::Continue));
        }
        // +/− pattern dialog: Apply / Cancel.
        if app.pattern_select_dialog.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                if let Some((apply_rect, cancel_rect)) =
                    crate::dialogs::pattern_select_dialog::button_rects(area)
                {
                    let (col, row) = (mouse_event.column, mouse_event.row);
                    if col >= apply_rect.x
                        && col < apply_rect.x + apply_rect.width
                        && row >= apply_rect.y
                        && row < apply_rect.y + apply_rect.height
                    {
                        return Ok(Some(AppAction::PatternSelectConfirm));
                    }
                    if col >= cancel_rect.x
                        && col < cancel_rect.x + cancel_rect.width
                        && row >= cancel_rect.y
                        && row < cancel_rect.y + cancel_rect.height
                    {
                        return Ok(Some(AppAction::PatternSelectCancel));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // Mkdir dialog: handle clicks on Create and Cancel buttons.
        if app.mkdir_dialog.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                if let Some((create_rect, cancel_rect)) =
                    crate::dialogs::mkdir_dialog::mkdir_button_rects(area)
                {
                    let (col, row) = (mouse_event.column, mouse_event.row);
                    if col >= create_rect.x
                        && col < create_rect.x + create_rect.width
                        && row >= create_rect.y
                        && row < create_rect.y + create_rect.height
                    {
                        return Ok(Some(AppAction::MkdirConfirm));
                    }
                    if col >= cancel_rect.x
                        && col < cancel_rect.x + cancel_rect.width
                        && row >= cancel_rect.y
                        && row < cancel_rect.y + cancel_rect.height
                    {
                        return Ok(Some(AppAction::MkdirCancel));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // Archive dialog: handle clicks on Create and Cancel buttons.
        if app.archive_dialog.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                if let Some((create_rect, cancel_rect)) =
                    crate::dialogs::archive_dialog::archive_button_rects(area)
                {
                    let (col, row) = (mouse_event.column, mouse_event.row);
                    if col >= create_rect.x
                        && col < create_rect.x + create_rect.width
                        && row >= create_rect.y
                        && row < create_rect.y + create_rect.height
                    {
                        return Ok(Some(AppAction::ArchiveConfirm));
                    }
                    if col >= cancel_rect.x
                        && col < cancel_rect.x + cancel_rect.width
                        && row >= cancel_rect.y
                        && row < cancel_rect.y + cancel_rect.height
                    {
                        return Ok(Some(AppAction::ArchiveCancel));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // New file error dialog: handle click on OK to close.
        if app.new_file_error.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                if let Some(ok_rect) = Renderer::new_file_error_ok_rect(area) {
                    let (col, row) = (mouse_event.column, mouse_event.row);
                    if col >= ok_rect.x
                        && col < ok_rect.x + ok_rect.width
                        && row >= ok_rect.y
                        && row < ok_rect.y + ok_rect.height
                    {
                        app.new_file_error = None;
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // New file dialog: handle clicks on Create and Cancel buttons.
        if app.new_file_dialog.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                if let Some((create_rect, cancel_rect)) =
                    crate::dialogs::new_file_dialog::new_file_button_rects(area)
                {
                    let (col, row) = (mouse_event.column, mouse_event.row);
                    if col >= create_rect.x
                        && col < create_rect.x + create_rect.width
                        && row >= create_rect.y
                        && row < create_rect.y + create_rect.height
                    {
                        return Ok(Some(AppAction::NewFileConfirm));
                    }
                    if col >= cancel_rect.x
                        && col < cancel_rect.x + cancel_rect.width
                        && row >= cancel_rect.y
                        && row < cancel_rect.y + cancel_rect.height
                    {
                        return Ok(Some(AppAction::NewFileCancel));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // Editor "Save changes?" dialog: handle clicks on option rows (1=Save, 2=Discard, 3=Cancel).
        if app.editor_confirm_pending {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                if let Some(rects) = crate::browser::editor::editor_confirm_option_rects(area) {
                    let (col, row) = (mouse_event.column, mouse_event.row);
                    for (opt_rect, choice) in rects {
                        if col >= opt_rect.x
                            && col < opt_rect.x + opt_rect.width
                            && row >= opt_rect.y
                            && row < opt_rect.y + opt_rect.height
                        {
                            return Ok(Some(AppAction::EditorConfirmChoice(choice)));
                        }
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // Help dialog: any mouse click closes.
        if app.help_dialog {
            if matches!(mouse_event.kind, MouseEventKind::Down(_)) {
                return Ok(Some(AppAction::HelpClose));
            }
            return Ok(Some(AppAction::Continue));
        }
        // Settings dialog: any mouse click closes.
        if app.settings_dialog.is_some() {
            if matches!(mouse_event.kind, MouseEventKind::Down(_)) {
                return Ok(Some(AppAction::SettingsClose));
            }
            return Ok(Some(AppAction::Continue));
        }
        // Size info dialog: any mouse click closes.
        if app.size_info_dialog.is_some() {
            if matches!(mouse_event.kind, MouseEventKind::Down(_)) {
                return Ok(Some(AppAction::SizeInfoClose));
            }
            return Ok(Some(AppAction::Continue));
        }
        // Find / panel overlays / F2 rename: modal — absorb mouse like F1 (no panel hit-test).
        if app.find_dialog.is_some() {
            return Ok(Some(AppAction::Continue));
        }
        if app.left_panel_settings_overlay.is_some()
            || app.right_panel_settings_overlay.is_some()
        {
            return Ok(Some(AppAction::Continue));
        }
        if app.rename_attr_dialog.is_some() {
            return Ok(Some(AppAction::Continue));
        }
        // Operation confirm dialog: handle clicks on Yes/No buttons (mouse/touchpad friendly).
        if let Some((op, _)) = app.operation_confirm_pending.as_ref() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                let show_paths = matches!(
                    op,
                    crate::app::state::Operation::Copy | crate::app::state::Operation::Move
                );
                if let Some((_dialog_rect, yes_rect, no_rect)) =
                    Renderer::operation_confirm_button_rects(area, show_paths)
                {
                    let (col, row) = (mouse_event.column, mouse_event.row);
                    if col >= yes_rect.x
                        && col < yes_rect.x + yes_rect.width
                        && row >= yes_rect.y
                        && row < yes_rect.y + yes_rect.height
                    {
                        return Ok(Some(AppAction::DeleteConfirmChoice(
                            DeleteConfirmChoice::Yes,
                        )));
                    }
                    if col >= no_rect.x
                        && col < no_rect.x + no_rect.width
                        && row >= no_rect.y
                        && row < no_rect.y + no_rect.height
                    {
                        return Ok(Some(AppAction::DeleteConfirmChoice(
                            DeleteConfirmChoice::No,
                        )));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        let panel_height = crate::util::compute_panel_height();
        let panels_mouse = Self::panels_and_menu_mouse_enabled(app);

        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                if panels_mouse {
                    app.active_panel_mut().move_up(panel_height);
                }
            }
            MouseEventKind::ScrollDown => {
                if panels_mouse {
                    app.active_panel_mut().move_down(panel_height);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if mouse_event.row == term_h.saturating_sub(1) {
                    if panels_mouse {
                        if let Some(action) =
                            Self::hit_test_menu_bar(mouse_event.column, term_w, app)
                        {
                            return Ok(Some(action));
                        }
                    }
                }
                if panels_mouse {
                    if let Some((panel_index, file_index)) = Self::hit_test_panel(
                        mouse_event.column,
                        mouse_event.row,
                        term_w,
                        term_h,
                        app,
                    ) {
                        app.focus_panel();
                        app.set_active_panel(panel_index);
                        let panel = if panel_index == 0 {
                            app.left_panel_mut()
                        } else {
                            app.right_panel_mut()
                        };
                        panel.set_selection(file_index, panel_height);

                        let now = std::time::Instant::now();
                        let is_double = app
                            .last_mouse_click
                            .as_ref()
                            .map(|(prev, p, f)| {
                                now.duration_since(*prev) < std::time::Duration::from_millis(400)
                                    && *p == panel_index
                                    && *f == file_index
                            })
                            .unwrap_or(false);
                        app.last_mouse_click = Some((now, panel_index, file_index));

                        if is_double {
                            let panel = if panel_index == 0 {
                                app.left_panel_mut()
                            } else {
                                app.right_panel_mut()
                            };
                            if let Some(file) = panel.get_selected_file() {
                                let name_lower = file.name.trim_end_matches('/').to_lowercase();
                                let is_zip = name_lower.ends_with(".zip");
                                if file.is_dir || is_zip {
                                    panel.enter_directory()?;
                                    app.sync_process_cwd_to_active_panel();
                                    return Ok(Some(AppAction::PanelNavigated));
                                } else if !file.is_parent_dir() && file.is_executable {
                                    let cmd = format!("./{}", file.name);
                                    return Ok(Some(AppAction::RunCommand(cmd)));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    /// Map mouse (col, row) to (panel_index, file_index) when clicking in a panel's file list.
    /// Layout must match ui::Renderer::draw_panels_view (frame border 1, inner, left/right split).
    /// Hit test menu bar (row 0). Returns AppAction for the clicked item.
    fn hit_test_menu_bar(
        col: u16,
        term_w: u16,
        app: &mut AppState,
    ) -> Option<AppAction> {
        let items = Renderer::menu_bar_items();
        let menu_item_count = items.len() as u16;
        if menu_item_count == 0 {
            return None;
        }
        let slot_w = term_w / menu_item_count;
        let slot_index = (col / slot_w).min(menu_item_count - 1) as usize;
        let (_, key) = items.get(slot_index)?;
        // In command prompt mode only F10 (Quit) is active.
        if app.focus == Focus::CommandLine && *key != 10 {
            return None;
        }
        match key {
            1 => Some(AppAction::OpenHelpDialog),
            2 => Some(AppAction::OpenRenameAttrDialog),
            3 => {
                if app
                    .active_panel_ref()
                    .get_selected_file()
                    .map_or(false, |f| !f.is_dir && !f.is_parent_dir())
                {
                    Some(AppAction::OpenViewer)
                } else {
                    None
                }
            }
            4 => {
                if crate::core::panel_backend::supports_edit(&app.get_current_location())
                    && app
                        .active_panel_ref()
                        .get_selected_file()
                        .map_or(false, |f| !f.is_dir && !f.is_parent_dir())
                {
                    Some(AppAction::OpenEditor)
                } else {
                    None
                }
            }
            5 => {
                let source = app.get_current_dir().to_string();
                let target = app.get_opposite_panel_dir().to_string();
                if source != target {
                    let (names, restore_after, restore_before) = app
                        .active_panel_mut()
                        .get_names_to_copy_with_restore_neighbors();
                    if !names.is_empty() {
                        let opposite = app.get_opposite_panel_location();
                        let (target_location, target_fs_path) = match &opposite {
                            crate::core::location::PanelLocation::Zip { .. } => {
                                (Some(opposite), None)
                            }
                            crate::core::location::PanelLocation::Fs(_) => {
                                (None, Some(app.get_opposite_panel_target_fs_path()))
                            }
                        };
                        let params = CopyParams {
                            source_dir: source,
                            target_dir: target,
                            source_location: Some(app.get_current_location()),
                            target_location,
                            target_fs_path,
                            items: names,
                            restore_selection_after: restore_after,
                            restore_selection_before: restore_before,
                        };
                        app.operation_confirm_pending =
                            Some((crate::app::state::Operation::Copy, params));
                        app.operation_confirm_focus_yes = true;
                        Some(AppAction::Continue)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            6 => {
                let source = app.get_current_dir().to_string();
                let target = app.get_opposite_panel_dir().to_string();
                if source != target {
                    let (names, restore_after, restore_before) = app
                        .active_panel_mut()
                        .get_names_to_copy_with_restore_neighbors();
                    if !names.is_empty() {
                        let opposite = app.get_opposite_panel_location();
                        let (target_location, target_fs_path) = match &opposite {
                            crate::core::location::PanelLocation::Zip { .. } => {
                                (Some(opposite), None)
                            }
                            crate::core::location::PanelLocation::Fs(_) => {
                                (None, Some(app.get_opposite_panel_target_fs_path()))
                            }
                        };
                        let params = CopyParams {
                            source_dir: source,
                            target_dir: target,
                            source_location: Some(app.get_current_location()),
                            target_location,
                            target_fs_path,
                            items: names,
                            restore_selection_after: restore_after,
                            restore_selection_before: restore_before,
                        };
                        app.operation_confirm_pending =
                            Some((crate::app::state::Operation::Move, params));
                        app.operation_confirm_focus_yes = true;
                        Some(AppAction::Continue)
                    } else {
                        app.focus_command_line();
                        None
                    }
                } else {
                    app.focus_command_line();
                    None
                }
            }
            7 => {
                if crate::core::panel_backend::supports_mkdir(&app.get_current_location()) {
                    Some(AppAction::OpenMkdirDialog)
                } else {
                    None
                }
            }
            8 => {
                let (names, restore_after, restore_before) = app
                    .active_panel_mut()
                    .get_names_to_copy_with_restore_neighbors();
                if !names.is_empty() {
                    app.operation_confirm_pending = Some((
                        crate::app::state::Operation::Delete,
                        CopyParams {
                            source_dir: app.get_current_dir().to_string(),
                            target_dir: String::new(),
                            source_location: Some(app.get_current_location()),
                            target_location: None,
                            target_fs_path: None,
                            items: names,
                            restore_selection_after: restore_after,
                            restore_selection_before: restore_before,
                        },
                    ));
                    app.operation_confirm_focus_yes = true;
                    Some(AppAction::Continue)
                } else {
                    None
                }
            }
            9 => Some(AppAction::OpenSettingsDialog),
            10 => Some(AppAction::Quit),
            _ => None,
        }
    }

    fn hit_test_panel(
        col: u16,
        row: u16,
        term_w: u16,
        term_h: u16,
        app: &AppState,
    ) -> Option<(usize, usize)> {
        let content_height = term_h.saturating_sub(2);
        let inner_x = 1u16;
        let inner_y = 1u16;
        let inner_w = term_w.saturating_sub(2);
        let inner_h = content_height.saturating_sub(2);
        let panel_content_height = inner_h.saturating_sub(1);
        let left_w = inner_w / 2;
        let right_w = inner_w.saturating_sub(left_w).saturating_sub(1);

        if row < inner_y || row >= inner_y + panel_content_height {
            return None;
        }
        let local_row = (row - inner_y) as usize;

        let panel_height = crate::util::compute_panel_height();

        // Left panel
        if col >= inner_x && col < inner_x + left_w {
            let panel = app.left_panel();
            let scroll = panel.get_scroll_offset();
            let files_len = panel.get_files().len();
            let file_index = match panel.get_view_mode() {
                crate::browser::panel::ViewMode::SingleColumn => scroll + local_row,
                crate::browser::panel::ViewMode::DoubleColumn => {
                    let in_right_col = col >= inner_x + left_w / 2;
                    if in_right_col {
                        scroll + panel_height + local_row
                    } else {
                        scroll + local_row
                    }
                }
            };
            if file_index < files_len {
                return Some((0, file_index));
            }
        }

        // Right panel
        if col >= inner_x + left_w + 1 && col < inner_x + left_w + 1 + right_w {
            let panel = app.right_panel();
            let scroll = panel.get_scroll_offset();
            let files_len = panel.get_files().len();
            let right_panel_x = inner_x + left_w + 1;
            let col_in_right = col - right_panel_x;
            let file_index = match panel.get_view_mode() {
                crate::browser::panel::ViewMode::SingleColumn => scroll + local_row,
                crate::browser::panel::ViewMode::DoubleColumn => {
                    let in_right_col = col_in_right >= right_w / 2;
                    if in_right_col {
                        scroll + panel_height + local_row
                    } else {
                        scroll + local_row
                    }
                }
            };
            if file_index < files_len {
                return Some((1, file_index));
            }
        }

        None
    }
}
