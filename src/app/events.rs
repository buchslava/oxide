use crate::app::ctrl_x_chord::{self, SuspendChordResult};
use crate::app::panel_refresh;
use crate::app::state::{
    AppState, CopyParams, Focus, Operation, RenameAttrDialogState, RenameAttrField,
    SizeInfoDialogState,
};
use crate::browser::clipboard;
use crate::browser::diff_viewer::{cancel_folder_compare_pending, handle_diff_key, handle_diff_mouse};
pub use crate::browser::editor::EditorConfirmChoice;
use crate::browser::editor::{handle_editor_key, handle_editor_mouse, paste_text_as_is};
use crate::browser::panel::PanelOperations;
use crate::browser::viewer::{handle_viewer_key, handle_viewer_mouse};
use crate::core::copy_state::same_folder_copy_dest_name;
use crate::core::location::PanelLocation;
use crate::core::panel_backend::{supports_edit, supports_mkdir, supports_new_file};
use crate::dialogs::{
    actions_dialog, archive_dialog, error_detail_dialog, find_dialog, mkdir_dialog, new_file_dialog,
    panel_overlay, pattern_select_dialog, rename_attr, settings_dialog, size_info_dialog,
};
use crate::ui::text_input;
use crate::util::compute_panel_height;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
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
    /// Ctrl+X then D: two marked non-dir files → full-screen diff; no marks → compare both panel dirs (C/S/X prefixes).
    OpenDiffViewer,
    /// ESC in diff viewer: close.
    DiffViewerClose,
    /// F7: open "Create a new Directory" dialog (MC-style).
    OpenMkdirDialog,
    /// Enter in mkdir dialog: create directory and close (name from dialog field).
    MkdirConfirm(String),
    /// ESC in mkdir dialog: cancel and close.
    MkdirCancel,
    /// Ctrl+X then A: open "Archive" dialog (create zip of selected items; originals kept).
    OpenArchiveDialog,
    /// Enter in archive dialog: create archive and close (name from dialog field).
    ArchiveConfirm(String),
    /// ESC in archive dialog: cancel and close.
    ArchiveCancel,
    /// ESC during archive progress: stop archiving and close progress dialog (like CopyCancel).
    ArchiveProgressCancel,
    /// Ctrl+X then N: open "New file" dialog (create empty file in current directory or archive).
    OpenNewFileDialog,
    /// Enter in new file dialog: create file and close (or show error if exists).
    NewFileConfirm(String),
    /// ESC in new file dialog: cancel and close.
    NewFileCancel,
    /// F2: open "Rename / Attributes" dialog (single file or group).
    OpenRenameAttrDialog,
    /// Enter in F2 dialog: apply rename + chmod and close.
    RenameAttrConfirm,
    /// ESC in F2 dialog: cancel and close.
    RenameAttrCancel,
    /// Ctrl+X then S: open "Size info" dialog (total size of selected files and folders).
    OpenSizeInfoDialog,
    /// ESC in size info dialog: close.
    SizeInfoClose,
    /// F1: open Actions dialog (Ctrl shortcuts as clickable rows).
    OpenActionsDialog,
    /// ESC, q, or click outside Actions dialog: close.
    ActionsClose,
    /// Ctrl+R style: refresh both panels (also used from F1 Actions).
    RefreshBothPanels,
    /// Ctrl+X R: refresh active panel only.
    RefreshActivePanel,
    /// Focus command line and copy/clear (Ctrl+C from Actions).
    CommandLineCopy,
    /// Focus command line and paste (Ctrl+V from Actions).
    CommandLinePaste,
    /// Close scrollable error details dialog.
    ErrorDetailClose,
    /// F9: open Settings dialog.
    OpenSettingsDialog,
    /// ESC or click outside Settings dialog: close.
    SettingsClose,
    /// Ctrl+X then 1: open Left panel settings overlay over the left panel.
    OpenLeftPanelSettings,
    /// Ctrl+X then 2: open Right panel settings overlay over the right panel.
    OpenRightPanelSettings,
    /// Close Left panel settings overlay.
    CloseLeftPanelSettings,
    /// Close Right panel settings overlay.
    CloseRightPanelSettings,
    /// Ctrl+X then F: open Find file dialog.
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
    /// Ctrl+X then H: toggle hidden files visibility.
    ToggleShowHidden,
    /// Ctrl+X then C: save left/right paths and active panel to settings.json and set `pinned_layout`
    /// so both paths restore on the next run even if autosave is off.
    PersistPanelState,
    /// Panel directory changed (Enter or double-click on dir). Used for autosave of panel cwds.
    PanelNavigated,
    /// A specific setting was toggled/changed in the F9 Settings dialog (or panel overlay). Main applies to persisted_settings, saves when autosave is on (or theme / autosave flag changed), then resyncs panels if needed.
    SettingChange(SettingChange),
    /// Ctrl+X then T toggled view mode; writes settings when autosave is on (or use Ctrl+X C).
    ViewModeToggled,
}

/// Single source of truth: each variant updates one field in PersistedSettings. Applied in main.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingChange {
    AutosaveToggle,
    /// Sync active panel to shell's cwd when returning from subshell (F9 Settings → General).
    SyncPanelToShellCwdToggle,
    /// Toggle auto-return to panels after running a command/executable.
    AutoReopenPanelsAfterCommandToggle,
    /// Cycle the delay (seconds) for auto-return to panels.
    AutoReopenPanelsAfterCommandDelayCycle,
    /// Toggle file name pattern mode for Find (Ctrl+X then F) and +/−: wildcards vs regex (F9 General).
    FilePatternModeCycle,
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
    /// F9 General: Safe delete (trash vs permanent); only applies when OS trash is available.
    SafeDeleteToggle,
    /// F9 Theme: apply preset at `ThemeId::ALL` index.
    ThemeSelect(usize),
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
            || app.settings_dialog.is_some()
            || app.actions_dialog.is_some()
            || app.error_detail.is_some()
            || app.find_dialog.is_some()
            || app.left_panel_settings_overlay.is_some()
            || app.right_panel_settings_overlay.is_some()
            || app.editor_confirm_pending
            || app.copy_progress.is_some()
            || app.archive_progress.is_some()
            || app.folder_compare_pending.is_some()
            || app.diff_viewer_screen.is_some()
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

    /// During post-command countdown: panels aren't visible yet, so ignore all input **except Esc**
    /// (Esc abandons the countdown and restores panels immediately).
    fn handle_events_post_command_countdown(app: &mut AppState) -> io::Result<AppAction> {
        while event::poll(std::time::Duration::ZERO)? {
            let ev = event::read()?;
            if matches!(ev, Event::Key(k) if k.code == KeyCode::Esc) {
                if let Some(cd) = app.post_command_countdown.as_mut() {
                    cd.reveal_at = std::time::Instant::now();
                }
            }
        }
        if !event::poll(std::time::Duration::from_millis(100))? {
            return Ok(AppAction::Continue);
        }
        let ev = event::read()?;
        if matches!(ev, Event::Key(k) if k.code == KeyCode::Esc) {
            if let Some(cd) = app.post_command_countdown.as_mut() {
                cd.reveal_at = std::time::Instant::now();
            }
        }
        Ok(AppAction::Continue)
    }

    pub fn handle_events(app: &mut AppState) -> io::Result<AppAction> {
        if app.post_command_countdown_active() {
            return Self::handle_events_post_command_countdown(app);
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
                // When diff viewer is open (Ctrl+D), it gets keys before the file viewer.
                if let Some(action) = handle_diff_key(app, key) {
                    return Ok(Some(action));
                }
                // When viewer is open, it gets keys first (e.g. opened from Find F3; Find stays open behind).
                if let Some(action) = handle_viewer_key(app, key) {
                    return Ok(Some(action));
                }
                // When editor "Save changes?" dialog is open: 1/2/3 direct, Tab/↑↓ cycle, Enter confirms, Esc=Cancel.
                if app.editor_confirm_pending {
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
                    let action = find_dialog::handle_key(app, key.code, key.modifiers)
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
                        return Ok(Some(AppAction::DeleteConfirmChoice(
                            confirm_choice,
                        )));
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
                            return Ok(Some(AppAction::CopyErrorChoice(
                                error_choice,
                            )));
                        }
                        _ => None,
                    };
                    if let Some(error_choice) = choice {
                        return Ok(Some(AppAction::CopyErrorChoice(
                            error_choice,
                        )));
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
                // Ctrl+X D panel-directory compare (no marks): modal until the worker finishes — Esc cancels.
                if app.folder_compare_pending.is_some() {
                    if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
                        cancel_folder_compare_pending(app);
                        return Ok(Some(AppAction::Continue));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // F7 "Create directory" dialog: modal — same as F1: never leak keys to panel/command line.
                if app.mkdir_dialog.is_some() {
                    return Ok(Some(
                        mkdir_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                if app.pattern_select_dialog.is_some() {
                    return Ok(Some(
                        pattern_select_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Ctrl+A "Archive" dialog
                if app.archive_dialog.is_some() {
                    return Ok(Some(
                        archive_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Ctrl+N "New file" dialog
                if app.new_file_dialog.is_some() {
                    return Ok(Some(
                        new_file_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Panel settings overlay (Ctrl+X then 1 left, 2 right)
                if app.left_panel_settings_overlay.is_some()
                    || app.right_panel_settings_overlay.is_some()
                {
                    return Ok(Some(
                        panel_overlay::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Scrollable error details (same keys as F1 Actions dialog).
                if app.error_detail.is_some() {
                    return Ok(Some(
                        error_detail_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // F1 Actions dialog: Esc/q close; all other keys are absorbed (modal).
                if app.actions_dialog.is_some() {
                    return Ok(Some(
                        actions_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // F9 Settings dialog
                if app.settings_dialog.is_some() {
                    return Ok(Some(
                        settings_dialog::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Size info: while scanning — modal (Esc / A = abort; mouse on [ Abort ]). Done: subshell chord, else close; Esc does not fall through (would focus command line).
                if app.size_info_dialog.is_some() {
                    let code_sz = match key.code {
                        KeyCode::Char('\t') => KeyCode::Tab,
                        KeyCode::Char('\u{7f}') => KeyCode::Backspace,
                        other => other,
                    };
                    let calculating = matches!(
                        app.size_info_dialog,
                        Some(SizeInfoDialogState::Calculating { .. })
                    );
                    if calculating {
                        match ctrl_x_chord::poll_suspend_chord(app, code_sz, key.modifiers) {
                            SuspendChordResult::Consumed => {
                                return Ok(Some(AppAction::Continue));
                            }
                            SuspendChordResult::SuspendToShell => {
                                return Ok(Some(AppAction::Suspend));
                            }
                            SuspendChordResult::NotHandled => {}
                        }
                        if matches!(code_sz, KeyCode::Esc)
                            || matches!(code_sz, KeyCode::Char('a' | 'A'))
                        {
                            return Ok(Some(AppAction::SizeInfoClose));
                        }
                        return Ok(Some(AppAction::Continue));
                    }
                    match ctrl_x_chord::poll_suspend_chord(app, code_sz, key.modifiers) {
                        SuspendChordResult::Consumed => {
                            return Ok(Some(AppAction::Continue));
                        }
                        SuspendChordResult::SuspendToShell => {
                            return Ok(Some(AppAction::Suspend));
                        }
                        SuspendChordResult::NotHandled => {}
                    }
                    // Esc closes the dialog only; do not fall through to panel (Esc would move focus to command line).
                    let esc_close_only = matches!(code_sz, KeyCode::Esc | KeyCode::Char('\x1b'));
                    size_info_dialog::close(app);
                    if esc_close_only {
                        return Ok(Some(AppAction::Continue));
                    }
                }
                // F2 "Rename / Attributes" dialog
                if app.rename_attr_dialog.is_some() {
                    if app.rename_attr_error.is_some() {
                        app.rename_attr_error = None;
                        return Ok(Some(AppAction::Continue));
                    }
                    return Ok(Some(
                        rename_attr::handle_key(app, key.code, key.modifiers)
                            .unwrap_or(AppAction::Continue),
                    ));
                }
                // Some terminals send Tab as Char('\t'); treat it as Tab.
                // DEL (0x7F) is occasionally reported as Char instead of KeyCode::Backspace.
                let code = match key.code {
                    KeyCode::Char('\t') => KeyCode::Tab,
                    KeyCode::Char('\u{7f}') => KeyCode::Backspace,
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
                let panel_height = compute_panel_height();
                if let Some(action) =
                    Self::handle_panel_direct_control_shortcuts(app, code, key.modifiers)
                {
                    return Ok(Some(action));
                }
                // Oxide shortcuts: Ctrl+X then a letter (e.g. F find, O shell).
                if app.ctrl_x_chord_pending {
                    app.ctrl_x_chord_pending = false;
                    if code == KeyCode::Esc {
                        return Ok(Some(AppAction::Continue));
                    }
                    if ctrl_x_chord::is_ctrl_x_prefix(code, key.modifiers) {
                        return Ok(Some(AppAction::Continue));
                    }
                    if let KeyCode::Char(c) = code {
                        let c = if c == '\x04' {
                            '\x04'
                        } else {
                            c.to_ascii_lowercase()
                        };
                        return Ok(Some(Self::handle_ctrl_key(app, c, panel_height)));
                    }
                } else if ctrl_x_chord::is_ctrl_x_prefix(code, key.modifiers) {
                    app.ctrl_x_chord_pending = true;
                    return Ok(Some(AppAction::Continue));
                }
                // Panel has focus: navigation, panel switch, or move to command line.
                match code {
                    KeyCode::Up => app.active_panel_mut().move_up(panel_height),
                    KeyCode::Down => app.active_panel_mut().move_down(panel_height),
                    KeyCode::Left => app.active_panel_mut().smart_move_left(panel_height),
                    KeyCode::Right => app.active_panel_mut().smart_move_right(panel_height),
                    KeyCode::PageUp => app.active_panel_mut().page_up(panel_height),
                    KeyCode::PageDown => app.active_panel_mut().page_down(panel_height),
                    KeyCode::Enter => {
                        let enter_result = {
                            let panel = app.active_panel_mut();
                            if let Some(file) = panel.get_selected_file() {
                                if !file.is_dir && file.is_executable {
                                    let cmd = format!("./{}", file.name);
                                    return Ok(Some(AppAction::RunCommand(cmd)));
                                }
                            }
                            panel.enter_directory()
                        };
                        match enter_result {
                            Ok(()) => {
                                app.sync_process_cwd_to_active_panel_if_no_autosave();
                                return Ok(Some(AppAction::PanelNavigated));
                            }
                            Err(e) => {
                                error_detail_dialog::open_from_io(
                                    app,
                                    "Could not open",
                                    e,
                                );
                                return Ok(Some(AppAction::Continue));
                            }
                        }
                    }
                    KeyCode::Backspace if !app.command_line.is_empty() => {
                        // Esc returns to the panel but leaves the prompt text; Linux often sends BS as Ctrl+H.
                        app.focus_command_line();
                        app.command_line_backspace();
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
                        } else if key.modifiers.contains(KeyModifiers::ALT)
                            && c.eq_ignore_ascii_case(&'h')
                        {
                            return Ok(Some(AppAction::ToggleShowHidden));
                        } else if text_input::is_ctrl_backspace(key.modifiers, c)
                            && !app.command_line.is_empty()
                        {
                            app.focus_command_line();
                            app.command_line_backspace();
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
                        let (items, _, _) = app
                            .active_panel_ref()
                            .get_names_to_copy_with_restore_neighbors();
                        let (restore_after, restore_before) =
                            app.active_panel_ref().restore_hints_after_copy();
                        if !items.is_empty() {
                            let target_names = if source == target {
                                Some(
                                    items
                                        .iter()
                                        .map(|(n, _)| same_folder_copy_dest_name(n))
                                        .collect(),
                                )
                            } else {
                                None
                            };
                            let opposite = app.get_opposite_panel_location();
                            let (target_location, target_fs_path) = match &opposite {
                                PanelLocation::Archive { .. } => (Some(opposite), None),
                                PanelLocation::Fs(_) => (
                                    None,
                                    Some(app.get_opposite_panel_target_fs_path()),
                                ),
                            };
                            return Ok(Some(AppAction::Copy(CopyParams {
                                source_dir: source,
                                target_dir: target,
                                source_location: Some(app.get_current_location()),
                                target_location,
                                target_fs_path,
                                items,
                                target_names,
                                restore_selection_after: restore_after,
                                restore_selection_before: restore_before,
                            })));
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
                                    PanelLocation::Archive { .. } => (Some(opposite), None),
                                    PanelLocation::Fs(_) => (
                                        None,
                                        Some(app.get_opposite_panel_target_fs_path()),
                                    ),
                                };
                                return Ok(Some(AppAction::Move(CopyParams {
                                    source_dir: source,
                                    target_dir: target,
                                    source_location: Some(app.get_current_location()),
                                    target_location,
                                    target_fs_path,
                                    items,
                                    target_names: None,
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
                    KeyCode::F(1) => return Ok(Some(AppAction::OpenActionsDialog)),
                    KeyCode::F(9) => return Ok(Some(AppAction::OpenSettingsDialog)),
                    KeyCode::F(7) => {
                        if supports_mkdir(&app.get_current_location()) {
                            return Ok(Some(AppAction::OpenMkdirDialog));
                        }
                    }
                    KeyCode::F(2) => {
                        if let Some(f) = app.active_panel_mut().get_selected_file() {
                            if !f.is_parent_dir() {
                                return Ok(Some(AppAction::OpenRenameAttrDialog));
                            }
                        }
                    }
                    KeyCode::F(4) => {
                        if supports_edit(&app.get_current_location()) {
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
                                Operation::Delete,
                                CopyParams {
                                    source_dir: app.get_current_dir().to_string(),
                                    target_dir: String::new(),
                                    source_location: Some(app.get_current_location()),
                                    target_location: None,
                                    target_fs_path: None,
                                    items,
                                    target_names: None,
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
                if handle_diff_mouse(app, mouse_event) {
                    return Ok(Some(AppAction::Continue));
                }
                if handle_viewer_mouse(app, mouse_event) {
                    return Ok(Some(AppAction::Continue));
                }
                if handle_editor_mouse(app, mouse_event) {
                    return Ok(Some(AppAction::Continue));
                }
                let action = crate::app::mouse::handle_mouse_event(app, mouse_event)?;
                Ok(Some(
                    action.unwrap_or(AppAction::Continue),
                ))
            }
            // Bracketed paste (e.g. Cmd+V on macOS): editor first if open, else focused dialog/command line.
            Event::Paste(data) => {
                if app.editor_screen.is_some() {
                    if let Some(action) = paste_text_as_is(app, &data) {
                        return Ok(Some(action));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                if Self::paste_into_focused_input(app, &data) {
                    return Ok(Some(AppAction::Continue));
                }
                if Self::modal_blocks_command_line_paste(app) {
                    return Ok(Some(AppAction::Continue));
                }
                Ok(None)
            }
            Event::Resize(_, _) => Ok(None),
            _ => Ok(None), // FocusGained, etc. - drain
        }
    }

    /// If a dialog text input or the command line has logical focus, insert `data` there and return true.
    fn paste_into_focused_input(
        app: &mut AppState,
        data: &str,
    ) -> bool {
        if let Some(d) = app.find_dialog.as_mut() {
            if d.focus <= 3 {
                let input = match d.focus {
                    0 => &mut d.start_dir_input,
                    1 => &mut d.file_pattern_input,
                    2 => &mut d.ignore_pattern_input,
                    3 => &mut d.content_pattern_input,
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
        if let Some(action) =
            Self::handle_command_line_direct_control_shortcuts(app, code, modifiers)
        {
            return action;
        }
        let panel_height = compute_panel_height();
        if app.ctrl_x_chord_pending {
            app.ctrl_x_chord_pending = false;
            if code == KeyCode::Esc {
                return AppAction::Continue;
            }
            if ctrl_x_chord::is_ctrl_x_prefix(code, modifiers) {
                return AppAction::Continue;
            }
            if let KeyCode::Char(c) = code {
                let c = if c == '\x04' {
                    '\x04'
                } else {
                    c.to_ascii_lowercase()
                };
                return Self::handle_ctrl_key(app, c, panel_height);
            }
        } else if ctrl_x_chord::is_ctrl_x_prefix(code, modifiers) {
            app.ctrl_x_chord_pending = true;
            return AppAction::Continue;
        }
        // Tab may be passed as KeyCode::Tab (normalized from Char('\t') in dispatch_event).
        match code {
            KeyCode::Char(c) => {
                // Linux: many terminals send the physical Backspace key as Ctrl+H (^H), same as a
                // real Ctrl+H chord — only there do we treat Ctrl+H as erase on the command line.
                // macOS: Backspace is usually KeyCode::Backspace (^?), so Ctrl+H can toggle hidden.
                if text_input::is_ctrl_backspace(modifiers, c) {
                    #[cfg(target_os = "macos")]
                    {
                        return AppAction::ToggleShowHidden;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        app.command_line_backspace();
                        if app.command_line.is_empty() {
                            app.focus_panel();
                        }
                        return AppAction::Continue;
                    }
                }
                if modifiers.contains(KeyModifiers::ALT) && c.eq_ignore_ascii_case(&'h') {
                    return AppAction::ToggleShowHidden;
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
            KeyCode::Delete => {
                app.command_line_delete_forward();
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

    /// **Ctrl+O** (shell / suspend) and **Ctrl+R** (refresh both panels), shared by panel and command line.
    /// Direct keys — no **Ctrl+X** prefix (Midnight Commander–style).
    fn try_direct_ctrl_o_suspend_or_ctrl_r_refresh(
        app: &mut AppState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<AppAction> {
        if ctrl_x_chord::is_direct_ctrl_o_suspend(code, modifiers) {
            app.ctrl_x_chord_pending = false;
            return Some(AppAction::Suspend);
        }
        if !modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match code {
            KeyCode::Char('\x12' | 'r' | 'R') => {
                app.ctrl_x_chord_pending = false;
                panel_refresh::refresh_both_panels_restore_selection(app, None, None);
                Some(AppAction::Continue)
            }
            _ => None,
        }
    }

    /// **Ctrl+O** / **Ctrl+R** / **Ctrl+C** / **Ctrl+V** without a Ctrl+X prefix (panel focus).
    /// Clears a stale Ctrl+X chord. C/V are absorbed on the panel like before the chord system.
    fn handle_panel_direct_control_shortcuts(
        app: &mut AppState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<AppAction> {
        if let Some(action) = Self::try_direct_ctrl_o_suspend_or_ctrl_r_refresh(app, code, modifiers)
        {
            return Some(action);
        }
        if !modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match code {
            KeyCode::Char('\x03' | 'c' | 'C' | '\x16' | 'v' | 'V') => {
                app.ctrl_x_chord_pending = false;
                Some(AppAction::Continue)
            }
            _ => None,
        }
    }

    /// **Ctrl+O** / **Ctrl+R** / **Ctrl+C** / **Ctrl+V** without a Ctrl+X prefix (command line).
    fn handle_command_line_direct_control_shortcuts(
        app: &mut AppState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<AppAction> {
        if let Some(action) = Self::try_direct_ctrl_o_suspend_or_ctrl_r_refresh(app, code, modifiers)
        {
            return Some(action);
        }
        if !modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match code {
            KeyCode::Char('\x03' | 'c' | 'C') => {
                app.ctrl_x_chord_pending = false;
                if !app.command_line.is_empty() {
                    clipboard::set(&app.command_line);
                } else {
                    app.command_line_clear();
                }
                Some(AppAction::Continue)
            }
            KeyCode::Char('\x16' | 'v' | 'V') => {
                app.ctrl_x_chord_pending = false;
                if let Some(s) = clipboard::get() {
                    app.command_line_insert_str(&s);
                }
                Some(AppAction::Continue)
            }
            _ => None,
        }
    }

    fn handle_ctrl_key(
        app: &mut AppState,
        c: char,
        panel_height: usize,
    ) -> AppAction {
        match c {
            '1' => AppAction::OpenLeftPanelSettings,
            '2' => AppAction::OpenRightPanelSettings,
            'o' => AppAction::Suspend,
            's' => {
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
                if supports_new_file(&app.get_current_location()) {
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
            'c' => AppAction::PersistPanelState,
            'd' | '\x04' => AppAction::OpenDiffViewer,
            _ => AppAction::Continue,
        }
    }
}
