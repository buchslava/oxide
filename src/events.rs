use std::io;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    terminal::size,
};
use crate::app_state::{AppState, CopyParams, Focus};
pub use crate::editor::EditorConfirmChoice;
use crate::editor::{handle_editor_key, handle_editor_mouse};
use crate::panel::PanelOperations;
use crate::ui::Renderer;
use crate::viewer::handle_viewer_key;

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
    /// F2: open "Rename / Attributes" dialog (single file or group).
    OpenRenameAttrDialog,
    /// Enter in F2 dialog: apply rename + chmod and close.
    RenameAttrConfirm,
    /// ESC in F2 dialog: cancel and close.
    RenameAttrCancel,
    /// F9: open "Size info" dialog (total size of selected files and folders).
    OpenSizeInfoDialog,
    /// ESC in size info dialog: close.
    SizeInfoClose,
    /// F1: open Settings dialog.
    OpenSettingsDialog,
    /// ESC or mouse click in Settings dialog: close.
    SettingsClose,
    /// Ctrl+H: toggle hidden files visibility.
    ToggleShowHidden,
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
    /// Process all pending events: handle Key/Mouse (update app, return action), ignore FocusGained/Resize.
    /// Never discards keys. Returns the last action from a key/mouse, or None if queue empty or only non-keys.
    pub fn process_queued_events(app: &mut AppState) -> io::Result<Option<AppAction>> {
        let mut last_action = None;
        while event::poll(std::time::Duration::ZERO)? {
            let ev = event::read()?;
            if let Some(action) = Self::dispatch_event(app, ev)? {
                last_action = Some(action);
            }
        }
        Ok(last_action)
    }

    pub fn handle_events(app: &mut AppState) -> io::Result<AppAction> {
        // Process all queued events first (no block). Handle every Key/Mouse; drain non-keys.
        // This ensures rapid keypresses when switching panels (e.g. Tab then Down) are all applied.
        let mut last_action = None;
        while event::poll(std::time::Duration::ZERO)? {
            let ev = event::read()?;
            if let Some(action) = Self::dispatch_event(app, ev)? {
                last_action = Some(action);
            }
        }
        if last_action.is_some() {
            return Ok(last_action.unwrap());
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
    /// - **Panel → command line:** Type a printable character (focus moves and char is inserted), or press **F6** (focus only).
    /// - **Command line → panel:** **Tab** or **Esc** (focus returns to active panel; command line text is kept).
    /// - **Between panels:** **Tab** when focus is Panel switches left/right panel; from command line Tab first returns focus to panel.
    /// - All key handling branches on `app.focus` first; no key is handled by both panel and command line.
    fn dispatch_event(
        app: &mut AppState,
        ev: Event,
    ) -> io::Result<Option<AppAction>> {
        match ev {
            Event::Key(key) => {
                // When viewer is open, delegate to viewer module (ESC closes).
                if let Some(action) = handle_viewer_key(app, key) {
                    return Ok(Some(action));
                }
                // When editor "Save changes?" dialog is open: 1/2/3 direct, Tab/↑↓ cycle, Enter confirms, Esc=Cancel.
                if app.editor_confirm_pending {
                    use crate::editor::EditorConfirmChoice;
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
                            return Ok(Some(AppAction::CopyOverwriteChoice(app.copy_overwrite_focus + 1)));
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
                    if let Some(c) = choice {
                        return Ok(Some(AppAction::DeleteConfirmChoice(c)));
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
                            let c = match app.copy_error_focus {
                                0 => CopyErrorChoice::Ignore,
                                1 => CopyErrorChoice::Cancel,
                                _ => CopyErrorChoice::IgnoreAll,
                            };
                            return Ok(Some(AppAction::CopyErrorChoice(c)));
                        }
                        _ => None,
                    };
                    if let Some(c) = choice {
                        return Ok(Some(AppAction::CopyErrorChoice(c)));
                    }
                    return Ok(Some(AppAction::Continue));
                }
                // When copy is in progress (no other dialog), Esc cancels the copy.
                if app.copy_in_progress.is_some()
                    && app.copy_overwrite_dialog.is_none()
                    && app.copy_error_dialog.is_none()
                {
                    if key.code == KeyCode::Esc {
                        return Ok(Some(AppAction::CopyCancel));
                    }
                }
                // F7 "Create directory" dialog: handle text input and Enter/ESC.
                if app.mkdir_dialog.is_some() {
                    if let Some(action) =
                        crate::mkdir_dialog::handle_key(app, key.code, key.modifiers)
                    {
                        return Ok(Some(action));
                    }
                }
                // F1 Settings dialog: Esc closes.
                if app.settings_dialog.is_some() {
                    if let Some(action) =
                        crate::settings_dialog::handle_key(app, key.code, key.modifiers)
                    {
                        return Ok(Some(action));
                    }
                }
                // Size info dialog: Esc closes.
                if app.size_info_dialog.is_some() {
                    if let Some(action) =
                        crate::size_info_dialog::handle_key(app, key.code, key.modifiers)
                    {
                        return Ok(Some(action));
                    }
                }
                // F2 "Rename / Attributes" dialog.
                if app.rename_attr_dialog.is_some() {
                    if app.rename_attr_error.is_some() {
                        app.rename_attr_error = None;
                        return Ok(Some(AppAction::Continue));
                    }
                    if let Some(action) =
                        crate::rename_attr::handle_key(app, key.code, key.modifiers)
                    {
                        return Ok(Some(action));
                    }
                }
                // Some terminals send Tab as Char('\t'); treat it as Tab.
                let code = match key.code {
                    KeyCode::Char('\t') => KeyCode::Tab,
                    other => other,
                };
                if app.focus == Focus::CommandLine {
                    return Ok(Some(Self::handle_command_line_key(app, code, key.modifiers)));
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
                    }
                    KeyCode::Char(' ') => {
                        // Space = toggle mark on the current file, then move selection down.
                        app.active_panel_mut().toggle_mark_and_move_next(panel_height);
                    }
                    KeyCode::Char(c) => {
                        if c == '*' {
                            app.active_panel_mut().invert_selection();
                        } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                            return Ok(Some(Self::handle_ctrl_key(app, c)));
                        } else if c.is_ascii() && !c.is_control() {
                            app.focus_command_line();
                            app.command_line_insert(c);
                        }
                    }
                    KeyCode::Tab => app.switch_panel()?,
                    KeyCode::F(5) => {
                        let source = app.get_current_dir().to_string();
                        let target = app.get_opposite_panel_dir().to_string();
                        if source != target {
                            let (items, restore_after, restore_before) =
                                app.active_panel_mut().get_names_to_copy_with_restore_neighbors();
                            if !items.is_empty() {
                                return Ok(Some(AppAction::Copy(CopyParams {
                                    source_dir: source,
                                    target_dir: target,
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
                            let (items, restore_after, restore_before) =
                                app.active_panel_mut().get_names_to_copy_with_restore_neighbors();
                            if !items.is_empty() {
                                return Ok(Some(AppAction::Move(CopyParams {
                                    source_dir: source,
                                    target_dir: target,
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
                    KeyCode::F(1) => return Ok(Some(AppAction::OpenSettingsDialog)),
                    KeyCode::F(7) => return Ok(Some(AppAction::OpenMkdirDialog)),
                    KeyCode::F(2) => return Ok(Some(AppAction::OpenRenameAttrDialog)),
                    KeyCode::F(4) => {
                        if let Some(file) = app.active_panel_mut().get_selected_file() {
                            if !file.is_dir && !file.is_parent_dir() {
                                return Ok(Some(AppAction::OpenEditor));
                            }
                        }
                    }
                    KeyCode::F(8) => {
                        let (items, restore_after, restore_before) =
                            app.active_panel_mut().get_names_to_copy_with_restore_neighbors();
                        if !items.is_empty() {
                            app.operation_confirm_pending = Some((
                                crate::app_state::Operation::Delete,
                                CopyParams {
                                    source_dir: app.get_current_dir().to_string(),
                                    target_dir: String::new(),
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
            _ => Ok(None), // FocusGained, Resize, etc. - drain
        }
    }

    fn handle_command_line_key(
        app: &mut AppState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> AppAction {
        // Tab may be passed as KeyCode::Tab (normalized from Char('\t') in dispatch_event).
        match code {
            KeyCode::Char(c) => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    if c == 'o' {
                        return AppAction::Suspend;
                    }
                    if c == 'c' {
                        app.command_line_clear();
                        return AppAction::Continue;
                    }
                    if c == 'h' {
                        return AppAction::ToggleShowHidden;
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

    fn handle_ctrl_key(app: &mut AppState, c: char) -> AppAction {
        match c {
            'o' => AppAction::Suspend,
            'g' => {
                let (items, ..) = app.active_panel_ref().get_names_to_copy_with_restore_neighbors();
                if !items.is_empty() {
                    AppAction::OpenSizeInfoDialog
                } else {
                    AppAction::Continue
                }
            }
            't' => {
                app.toggle_view_mode();
                AppAction::Continue
            }
            'h' => AppAction::ToggleShowHidden,
            'r' => {
                let _ = app.active_panel_mut().refresh_files();
                AppAction::Continue
            }
            _ => AppAction::Continue,
        }
    }

    /// Returns Ok(Some(action)) when an action (e.g. RunCommand) should be handled by the main loop.
    fn handle_mouse_event(app: &mut AppState, mouse_event: MouseEvent) -> io::Result<Option<AppAction>> {
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
                        let c = match opt_row {
                            0 => CopyErrorChoice::Ignore,
                            1 => CopyErrorChoice::Cancel,
                            _ => CopyErrorChoice::IgnoreAll,
                        };
                        return Ok(Some(AppAction::CopyErrorChoice(c)));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        // Mkdir dialog: handle clicks on Create and Cancel buttons.
        if app.mkdir_dialog.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                if let Some((create_rect, cancel_rect)) = crate::mkdir_dialog::mkdir_button_rects(area) {
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
        // Editor "Save changes?" dialog: handle clicks on option rows (1=Save, 2=Discard, 3=Cancel).
        if app.editor_confirm_pending {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                if let Some(rects) = crate::editor::editor_confirm_option_rects(area) {
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
        // Operation confirm dialog: handle clicks on Yes/No buttons (mouse/touchpad friendly).
        if let Some((op, _)) = app.operation_confirm_pending.as_ref() {
            if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                let show_paths = matches!(op, crate::app_state::Operation::Copy | crate::app_state::Operation::Move);
                if let Some((_dialog_rect, yes_rect, no_rect)) =
                    Renderer::operation_confirm_button_rects(area, show_paths)
                {
                    let (col, row) = (mouse_event.column, mouse_event.row);
                    if col >= yes_rect.x
                        && col < yes_rect.x + yes_rect.width
                        && row >= yes_rect.y
                        && row < yes_rect.y + yes_rect.height
                    {
                        return Ok(Some(AppAction::DeleteConfirmChoice(DeleteConfirmChoice::Yes)));
                    }
                    if col >= no_rect.x
                        && col < no_rect.x + no_rect.width
                        && row >= no_rect.y
                        && row < no_rect.y + no_rect.height
                    {
                        return Ok(Some(AppAction::DeleteConfirmChoice(DeleteConfirmChoice::No)));
                    }
                }
            }
            return Ok(Some(AppAction::Continue));
        }
        let panel_height = crate::util::compute_panel_height();
        let in_panels_view = app.viewer_screen.is_none()
            && app.editor_screen.is_none()
            && app.copy_overwrite_dialog.is_none()
            && app.copy_error_dialog.is_none()
            && app.operation_confirm_pending.is_none()
            && app.mkdir_dialog.is_none()
            && app.rename_attr_dialog.is_none()
            && app.size_info_dialog.is_none()
            && app.settings_dialog.is_none();

        match mouse_event.kind {
            MouseEventKind::ScrollUp => app.active_panel_mut().move_up(panel_height),
            MouseEventKind::ScrollDown => app.active_panel_mut().move_down(panel_height),
            MouseEventKind::Down(MouseButton::Left) => {
                if mouse_event.row == term_h.saturating_sub(1) {
                    if let Some(action) = Self::hit_test_menu_bar(mouse_event.column, term_w, app) {
                        return Ok(Some(action));
                    }
                }
                if in_panels_view {
                    if let Some((panel_index, file_index)) =
                        Self::hit_test_panel(mouse_event.column, mouse_event.row, term_w, term_h, app)
                    {
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
                                if file.is_dir {
                                    panel.enter_directory()?;
                                    app.sync_process_cwd_to_active_panel();
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
    fn hit_test_menu_bar(col: u16, term_w: u16, app: &mut AppState) -> Option<AppAction> {
        let items = Renderer::menu_bar_items();
        let n = items.len() as u16;
        if n == 0 {
            return None;
        }
        let slot_w = term_w / n;
        let slot_index = (col / slot_w).min(n - 1) as usize;
        let (_, key) = items.get(slot_index)?;
        match key {
                    1 => Some(AppAction::OpenSettingsDialog),
                    2 => Some(AppAction::OpenRenameAttrDialog),
                    3 => {
                        if app.active_panel_ref().get_selected_file().map_or(false, |f| !f.is_dir && !f.is_parent_dir()) {
                            Some(AppAction::OpenViewer)
                        } else {
                            None
                        }
                    }
                    4 => {
                        if app.active_panel_ref().get_selected_file().map_or(false, |f| !f.is_dir && !f.is_parent_dir()) {
                            Some(AppAction::OpenEditor)
                        } else {
                            None
                        }
                    }
                    5 => {
                        let source = app.get_current_dir().to_string();
                        let target = app.get_opposite_panel_dir().to_string();
                        if source != target {
                            let (names, restore_after, restore_before) =
                                app.active_panel_mut().get_names_to_copy_with_restore_neighbors();
                            if !names.is_empty() {
                                let params = CopyParams {
                                    source_dir: source,
                                    target_dir: target,
                                    items: names,
                                    restore_selection_after: restore_after,
                                    restore_selection_before: restore_before,
                                };
                                app.operation_confirm_pending = Some((crate::app_state::Operation::Copy, params));
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
                            let (names, restore_after, restore_before) =
                                app.active_panel_mut().get_names_to_copy_with_restore_neighbors();
                            if !names.is_empty() {
                                let params = CopyParams {
                                    source_dir: source,
                                    target_dir: target,
                                    items: names,
                                    restore_selection_after: restore_after,
                                    restore_selection_before: restore_before,
                                };
                                app.operation_confirm_pending = Some((crate::app_state::Operation::Move, params));
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
                    7 => Some(AppAction::OpenMkdirDialog),
                    8 => {
                        let (names, restore_after, restore_before) =
                            app.active_panel_mut().get_names_to_copy_with_restore_neighbors();
                        if !names.is_empty() {
                            app.operation_confirm_pending = Some((
                                crate::app_state::Operation::Delete,
                                CopyParams {
                                    source_dir: app.get_current_dir().to_string(),
                                    target_dir: String::new(),
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
                crate::panel::ViewMode::SingleColumn => scroll + local_row,
                crate::panel::ViewMode::DoubleColumn => {
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
                crate::panel::ViewMode::SingleColumn => scroll + local_row,
                crate::panel::ViewMode::DoubleColumn => {
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
