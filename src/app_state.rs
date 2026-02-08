use std::io;
use std::sync::mpsc;
use ratatui::layout::Rect;
use ratatui_code_editor::editor::Editor;
use crate::panel::{Panel, PanelOperations, ViewMode};

/// Single source of truth for input target: panel (navigation) or command line (typing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Panel,
    CommandLine,
}

/// Copy, Move, or Delete operation (F5 / F6 / F8). Shared flow for progress, overwrite, and error dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Copy,
    Move,
    Delete,
}

/// Progress shown during Copy/Move. Full path of current file, position (e.g. "3 / 5").
#[derive(Debug, Clone)]
pub struct CopyProgress {
    pub operation: Operation,
    /// Full path of the file/dir being copied/moved (source).
    pub current_path: String,
    pub current: usize,
    pub total: usize,
}

/// Parameters for a copy operation: source dir, target dir, list of (name, is_dir).
/// restore_selection_after/before: after delete/move, try to select this file (after first, else before).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyParams {
    pub source_dir: String,
    pub target_dir: String,
    pub items: Vec<(String, bool)>,
    pub restore_selection_after: Option<String>,
    pub restore_selection_before: Option<String>,
}

/// In-progress copy/move state: operation, current index, "rewrite all" / "skip all" / "ignore all errors" flags.
#[derive(Debug, Clone)]
pub struct CopyInProgress {
    pub operation: Operation,
    pub params: CopyParams,
    pub current_index: usize,
    pub overwrite_all: bool,
    pub skip_all: bool,
    /// When true, future copy/move errors are skipped without showing the error dialog.
    pub ignore_all_errors: bool,
}

pub struct AppState {
    active_panel: usize,
    left_panel: Panel,
    right_panel: Panel,
    pub focus: Focus,
    pub command_line: String,
    pub command_line_cursor: usize,
    /// When Some, the Copy progress overlay is shown (current file, X of Y).
    pub copy_progress: Option<CopyProgress>,
    /// When Some, we're in the middle of F5 Copy; run_copy_step advances it.
    pub copy_in_progress: Option<CopyInProgress>,
    /// When Some (filename), show "file exists" overwrite dialog; user picks 1–4.
    pub copy_overwrite_dialog: Option<String>,
    /// Overwrite dialog: focused option index 0–4 (Rewrite, Rewrite all, Skip, Skip all, Cancel).
    pub copy_overwrite_focus: usize,
    /// When Some, a copy error occurred; user picks Ignore or Cancel.
    pub copy_error_dialog: Option<CopyErrorState>,
    /// Error dialog: focused option index 0–2 (Skip, Cancel, Ignore all).
    pub copy_error_focus: usize,
    /// When Some, show operation confirmation (Copy/Move/Delete) before proceeding.
    /// Used for: F8 Delete, mouse Delete, mouse Copy, mouse Move.
    pub operation_confirm_pending: Option<(Operation, CopyParams)>,
    /// When operation confirm dialog is open: true = Yes focused (Tab default), false = No.
    pub operation_confirm_focus_yes: bool,
    /// When Some, a directory delete is running in a background thread; poll for result to keep UI responsive.
    pub delete_pending_rx: Option<mpsc::Receiver<io::Result<()>>>,
    /// When set, copy/move/delete just completed; refresh source panel with (after, before) and clear.
    pub source_panel_restore: Option<(String, Option<String>, Option<String>)>,
    /// When Some, the embedded code editor is open (F4). None = panels view.
    pub editor_screen: Option<EditorScreenState>,
    /// When true, show "Save changes?" (1=Save, 2=Discard, 3/Esc=Cancel) before exiting editor.
    pub editor_confirm_pending: bool,
    /// Editor confirm dialog: focused option index 0=Save, 1=Discard, 2=Cancel.
    pub editor_confirm_focus: usize,
    /// When Some, the file viewer is open (F3). None = panels or editor view.
    pub viewer_screen: Option<ViewerScreenState>,
    /// When Some, F7 "Create directory" dialog is open (text field for new folder name).
    pub mkdir_dialog: Option<MkdirDialogState>,
    /// When Some, F2 "Rename / Attributes" dialog is open (single file: name + attrs; group: attrs only).
    pub rename_attr_dialog: Option<RenameAttrDialogState>,
    /// When Some, an error alert is shown on top of the F2 dialog (message to display).
    pub rename_attr_error: Option<String>,
    /// When Some, F9 "Size info" dialog is open (total size of selected items).
    pub size_info_dialog: Option<SizeInfoDialogState>,
    /// When Some, F1 Settings dialog is open (placeholder).
    pub settings_dialog: Option<()>,
    /// Receiver for background size calculation; polled in main loop.
    pub size_info_pending_rx: Option<mpsc::Receiver<SizeInfoProgress>>,
    /// Last left-click (instant, panel_index, file_index) for double-click detection.
    pub last_mouse_click: Option<(std::time::Instant, usize, usize)>,
    /// Last mouse (column, row) from any mouse event (scroll, move, click).
    pub last_mouse_position: Option<(u16, u16)>,
}

/// Message from background size-calculation thread.
#[derive(Debug, Clone)]
pub enum SizeInfoProgress {
    /// One more item processed.
    Progress {
        current: usize,
        total: usize,
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
        current_name: String,
    },
    /// Calculation complete.
    Done {
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
}

/// State for F9 "Size info" dialog. Shows progress during calculation, then final result.
#[derive(Debug, Clone)]
pub enum SizeInfoDialogState {
    /// Calculation in progress; show progress bar.
    Calculating {
        current: usize,
        total: usize,
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
        current_name: String,
    },
    /// Calculation complete; show final size.
    Done {
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
}

/// State for F7 "Create a new Directory" dialog (MC-style). Single text field for the new folder name.
/// focus: 0 = textarea, 1 = Create, 2 = Cancel.
#[derive(Debug, Clone)]
pub struct MkdirDialogState {
    pub name: String,
    pub cursor: usize,
    pub focus: usize,
}

/// Which part of the F2 dialog has focus (name field, permission checkboxes, user list, or group list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameAttrField {
    Name,
    Permissions,
    User,
    Group,
}

/// State for F2 "Rename / Attributes" dialog. Permissions as 12 checkboxes; owner/group as list selection.
#[derive(Debug, Clone)]
pub enum RenameAttrDialogState {
    Single {
        name: String,
        name_cursor: usize,
        /// Unix mode (0o7777: suid, sgid, sticky + rwx).
        mode: u32,
        /// Index into the 12 permission checkboxes (0..12).
        perm_focus: usize,
        owner: String,
        group: String,
        user_list: Vec<String>,
        group_list: Vec<String>,
        user_index: usize,
        group_index: usize,
        cwd: String,
        old_name: String,
        focus: RenameAttrField,
    },
    Group {
        count: usize,
        mode: u32,
        perm_focus: usize,
        owner: String,
        group: String,
        user_list: Vec<String>,
        group_list: Vec<String>,
        user_index: usize,
        group_index: usize,
        cwd: String,
        items: Vec<(String, bool)>,
        focus: RenameAttrField,
    },
}

/// State when the file viewer is open (F3). Text and hex modes.
pub struct ViewerScreenState {
    pub file_path: String,
    /// Raw file bytes (for hex mode; text mode uses same buffer decoded).
    pub content: Vec<u8>,
    /// Display mode: text (lines) or hex dump.
    pub view_mode: ViewerMode,
    /// First visible line index (scroll offset).
    pub scroll: usize,
    /// Last draw area (for consistent layout).
    pub area: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerMode {
    Text,
    Hex,
}

/// State when the embedded code editor is open (F4).
pub struct EditorScreenState {
    pub file_path: String,
    /// Content when file was opened; used to detect unsaved changes.
    pub initial_content: String,
    pub editor: Editor,
    /// Last draw area for the editor (used for input/mouse). When search is open, height is reduced by 1.
    pub area: Rect,
    /// When Some, search bar is open and the string is the current query (Ctrl+F).
    pub search_query: Option<String>,
    /// Toggle with F3: when true, Up/Down/PageUp/PageDown extend selection (editor only).
    pub selection_extend_mode: bool,
}

#[derive(Debug, Clone)]
pub struct CopyErrorState {
    pub operation: Operation,
    pub message: String,
}

impl AppState {
    pub fn new() -> io::Result<Self> {
        let current_dir = std::env::current_dir()?;
        let current_dir_str = current_dir.to_string_lossy().to_string();

        Ok(Self {
            active_panel: 0,
            left_panel: Panel::new(current_dir_str.clone())?,
            right_panel: Panel::new(current_dir_str)?,
            focus: Focus::Panel,
            command_line: String::new(),
            command_line_cursor: 0,
            copy_progress: None,
            copy_in_progress: None,
            copy_overwrite_dialog: None,
            copy_overwrite_focus: 0,
            copy_error_dialog: None,
            copy_error_focus: 0,
            operation_confirm_pending: None,
            operation_confirm_focus_yes: true,
            delete_pending_rx: None,
            source_panel_restore: None,
            editor_screen: None,
            editor_confirm_pending: false,
            editor_confirm_focus: 0,
            viewer_screen: None,
            mkdir_dialog: None,
            rename_attr_dialog: None,
            rename_attr_error: None,
            size_info_dialog: None,
            settings_dialog: None,
            size_info_pending_rx: None,
            last_mouse_click: None,
            last_mouse_position: None,
        })
    }

    /// Set the active panel by index (0 = left, 1 = right).
    pub fn set_active_panel(&mut self, panel_index: usize) {
        self.active_panel = if panel_index == 0 { 0 } else { 1 };
    }

    pub fn close_rename_attr_dialog(&mut self) {
        self.rename_attr_dialog = None;
        self.rename_attr_error = None;
    }

    /// Open F7 "Create directory" dialog with optional default name (e.g. from selected file, MC auto_fill_mkdir_name).
    pub fn open_mkdir_dialog(&mut self, default_name: String) {
        let cursor = default_name.len();
        self.mkdir_dialog = Some(MkdirDialogState {
            name: default_name,
            cursor,
            focus: 0,
        });
    }

    pub fn mkdir_dialog_insert(&mut self, c: char) {
        if let Some(ref mut d) = self.mkdir_dialog {
            let at = d.cursor.min(d.name.len());
            d.name.insert(at, c);
            d.cursor = at + 1;
        }
    }

    pub fn mkdir_dialog_backspace(&mut self) {
        if let Some(ref mut d) = self.mkdir_dialog {
            if d.cursor > 0 && d.cursor <= d.name.len() {
                d.name.remove(d.cursor - 1);
                d.cursor -= 1;
            }
        }
    }

    pub fn mkdir_dialog_move_left(&mut self) {
        if let Some(ref mut d) = self.mkdir_dialog {
            if d.cursor > 0 {
                d.cursor -= 1;
            }
        }
    }

    pub fn mkdir_dialog_move_right(&mut self) {
        if let Some(ref mut d) = self.mkdir_dialog {
            if d.cursor < d.name.len() {
                d.cursor += 1;
            }
        }
    }

    /// Close dialog and return the entered name (caller creates dir if non-empty).
    pub fn take_mkdir_dialog(&mut self) -> Option<String> {
        self.mkdir_dialog.take().map(|d| d.name)
    }

    pub fn close_mkdir_dialog(&mut self) {
        self.mkdir_dialog = None;
    }

    pub fn active_panel_mut(&mut self) -> &mut Panel {
        match self.active_panel {
            0 => &mut self.left_panel,
            1 => &mut self.right_panel,
            _ => &mut self.left_panel,
        }
    }

    pub fn left_panel(&self) -> &Panel {
        &self.left_panel
    }

    pub fn right_panel(&self) -> &Panel {
        &self.right_panel
    }

    pub fn left_panel_mut(&mut self) -> &mut Panel {
        &mut self.left_panel
    }

    pub fn right_panel_mut(&mut self) -> &mut Panel {
        &mut self.right_panel
    }

    pub fn active_panel(&self) -> usize {
        self.active_panel
    }

    /// Reference to the active panel (for read-only access).
    pub fn active_panel_ref(&self) -> &Panel {
        match self.active_panel {
            0 => &self.left_panel,
            _ => &self.right_panel,
        }
    }

    pub fn get_current_dir(&self) -> &str {
        match self.active_panel {
            0 => self.left_panel.get_current_dir(),
            1 => self.right_panel.get_current_dir(),
            _ => self.left_panel.get_current_dir(),
        }
    }

    /// Directory of the panel opposite to the active one (target for F5 Copy).
    pub fn get_opposite_panel_dir(&self) -> &str {
        match self.active_panel {
            0 => self.right_panel.get_current_dir(),
            1 => self.left_panel.get_current_dir(),
            _ => self.right_panel.get_current_dir(),
        }
    }

    /// Sync the process current directory to the active panel's directory.
    /// Call after any panel navigation (Enter on dir, ..) so that Ctrl+O shell and command line use the same cwd.
    pub fn sync_process_cwd_to_active_panel(&self) {
        if let Err(e) = std::env::set_current_dir(self.get_current_dir()) {
            eprintln!("Failed to change directory: {}", e);
        }
    }

    pub fn toggle_view_mode(&mut self) {
        let panel = self.active_panel_mut();
        let new_mode = match panel.get_view_mode() {
            ViewMode::SingleColumn => ViewMode::DoubleColumn,
            ViewMode::DoubleColumn => ViewMode::SingleColumn,
        };
        panel.set_view_mode(new_mode);
    }

    pub fn focus_command_line(&mut self) {
        self.focus = Focus::CommandLine;
    }

    pub fn focus_panel(&mut self) {
        self.focus = Focus::Panel;
    }

    pub fn command_line_insert(&mut self, c: char) {
        let at = self.command_line_cursor.min(self.command_line.len());
        self.command_line.insert(at, c);
        self.command_line_cursor = at + 1;
    }

    pub fn command_line_backspace(&mut self) {
        if self.command_line_cursor > 0 && self.command_line_cursor <= self.command_line.len() {
            self.command_line.remove(self.command_line_cursor - 1);
            self.command_line_cursor -= 1;
        }
    }

    pub fn command_line_move_left(&mut self) {
        if self.command_line_cursor > 0 {
            self.command_line_cursor -= 1;
        }
    }

    pub fn command_line_move_right(&mut self) {
        if self.command_line_cursor < self.command_line.len() {
            self.command_line_cursor += 1;
        }
    }

    pub fn command_line_clear(&mut self) {
        self.command_line.clear();
        self.command_line_cursor = 0;
    }

    pub fn take_command_line(&mut self) -> String {
        let cmd = std::mem::take(&mut self.command_line);
        self.command_line_cursor = 0;
        cmd
    }

    pub fn switch_panel(&mut self) -> io::Result<()> {
        let new_active_panel = if self.active_panel == 0 { 1 } else { 0 };
        let target_dir = if new_active_panel == 0 {
            self.left_panel.get_current_dir()
        } else {
            self.right_panel.get_current_dir()
        };
        if let Err(e) = std::env::set_current_dir(target_dir) {
            eprintln!("Failed to change directory: {}", e);
        }
        self.active_panel = new_active_panel;
        Ok(())
    }
}
