use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use ratatui::layout::Rect;
use ratatui_code_editor::editor::Editor;
use crate::location::PanelLocation;
use crate::panel::{Panel, PanelOperations, ViewMode};
use crate::settings::PersistedSettings;

/// Single source of truth for input target: panel (navigation), command line (typing), or a modal dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Panel,
    CommandLine,
    /// Find file dialog (Ctrl+F) has focus; panel and command line do not receive keys.
    FindDialog,
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
/// source_location: when Some, use panel_backend (handles Zip and Fs); when None, use legacy copy_ops with source_dir.
/// restore_selection_after/before: after delete/move, try to select this file (after first, else before).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyParams {
    pub source_dir: String,
    pub target_dir: String,
    /// When Some, use panel_backend for copy/move/delete (supports Zip).
    pub source_location: Option<PanelLocation>,
    /// When Some, use this as target path for panel_backend copy/move (e.g. when opposite panel is Zip, this is archive's parent).
    pub target_fs_path: Option<PathBuf>,
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
    /// When Some, the file viewer is open (F3). Loading = reading file in background; Ready = content available. None = panels or editor view.
    pub viewer_screen: Option<ViewerState>,
    /// When Some, F7 "Create directory" dialog is open (text field for new folder name).
    pub mkdir_dialog: Option<MkdirDialogState>,
    /// When Some, F2 "Rename / Attributes" dialog is open (single file: name + attrs; group: attrs only).
    pub rename_attr_dialog: Option<RenameAttrDialogState>,
    /// When Some, an error alert is shown on top of the F2 dialog (message to display).
    pub rename_attr_error: Option<String>,
    /// When Some, F9 "Size info" dialog is open (total size of selected items).
    pub size_info_dialog: Option<SizeInfoDialogState>,
    /// When Some, Ctrl+F "Find file" dialog is open.
    pub find_dialog: Option<FindDialogState>,
    /// When Some, F1 Settings dialog is open (two-column: sections list + content).
    pub settings_dialog: Option<SettingsDialogState>,
    /// When Some, Ctrl+Q "Left panel settings" overlay is open over the left panel.
    pub left_panel_settings_overlay: Option<PanelSettingsOverlayState>,
    /// When Some, Ctrl+W "Right panel settings" overlay is open over the right panel.
    pub right_panel_settings_overlay: Option<PanelSettingsOverlayState>,
    /// Last frame's left panel area (set by UI renderer); used to position left panel overlay.
    pub left_panel_rect: Option<Rect>,
    /// Last frame's right panel area; used to position right panel overlay.
    pub right_panel_rect: Option<Rect>,
    /// Receiver for background size calculation; polled in main loop.
    pub size_info_pending_rx: Option<mpsc::Receiver<SizeInfoProgress>>,
    /// Receiver for find file search thread; polled when find_dialog is open.
    pub find_search_rx: Option<mpsc::Receiver<FindMessage>>,
    /// Last left-click (instant, panel_index, file_index) for double-click detection.
    pub last_mouse_click: Option<(std::time::Instant, usize, usize)>,
    /// Last mouse (column, row) from any mouse event (scroll, move, click).
    pub last_mouse_position: Option<(u16, u16)>,
    /// When true, show hidden files (names starting with "."). Default true. Toggled by Ctrl+H.
    pub show_hidden_files: bool,
    /// Last saved/loaded settings from ~/.oxide/settings.json. Used to persist on change and for autosave.
    pub persisted_settings: PersistedSettings,
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
    },
    /// Calculation complete; show final size.
    Done {
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
}

/// State for F1 Settings dialog. Only UI navigation; all setting values live in PersistedSettings (single source of truth).
#[derive(Debug, Clone)]
pub struct SettingsDialogState {
    /// Selected section index: 0 General, 1 Left panel, 2 Right panel, 3 Info, 4 Help.
    pub selected_section: usize,
    /// true = focus on left list, false = focus on right content.
    pub focus_left: bool,
    /// When focus_left is false and section is Left/Right panel: 0 = View, 1 = Sort, 2 = Folders first, 3 = Show hidden.
    pub content_focus: usize,
}

impl Default for SettingsDialogState {
    fn default() -> Self {
        Self {
            selected_section: 0,
            focus_left: true,
            content_focus: 0,
        }
    }
}

/// State for panel settings overlay. content_focus: 0 = View, 1 = Sort, 2 = Folders first, 3 = Show hidden.
#[derive(Debug, Clone)]
pub struct PanelSettingsOverlayState {
    pub content_focus: usize,
}

/// One result from Find file: path and optional line number (when content search matched).
#[derive(Debug, Clone)]
pub struct FindResult {
    pub path: PathBuf,
    pub line: Option<u64>,
}

/// Message from the find search background thread.
#[derive(Debug)]
pub enum FindMessage {
    Match(PathBuf, Option<u64>),
    /// Current directory being traversed (empty when search is done).
    CurrentDir(String),
    Done,
}

/// Phase of the Find file dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindDialogPhase {
    /// Parameter form: start dir, pattern, content, options.
    Parameter,
    /// Search running; results accumulating.
    Searching,
    /// Search done; showing results list.
    Results,
}

/// State for Ctrl+F Find file dialog.
#[derive(Debug, Clone)]
pub struct FindDialogState {
    pub phase: FindDialogPhase,
    pub start_dir: String,
    pub start_dir_cursor: usize,
    pub file_pattern: String,
    pub file_pattern_cursor: usize,
    pub content_pattern: String,
    pub content_pattern_cursor: usize,
    pub recursive: bool,
    pub file_case_sens: bool,
    pub content_case_sens: bool,
    pub skip_hidden: bool,
    pub results: Vec<FindResult>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub status_message: String,
    /// Current directory being searched (shown during search; cleared when done).
    pub search_current_dir: String,
    /// Visible list rows (set from dialog inner height in draw). Used for scroll math.
    pub visible_list_rows: usize,
    /// Focus in parameter form: 0=start_dir, 1=file_pattern, 2=content, 3=recursive, 4=file_case, 5=content_case, 6=skip_hidden, 7=Find, 8=Cancel.
    pub focus: usize,
}

impl Default for PanelSettingsOverlayState {
    fn default() -> Self {
        Self { content_focus: 0 }
    }
}

/// Section indices for the Settings dialog sidebar.
pub const SETTINGS_SECTIONS: [&str; 5] = [
    "General settings",
    "Left panel",
    "Right panel",
    "Info",
    "Help",
];

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
        items: Vec<(String, bool)>,
        focus: RenameAttrField,
    },
}

/// Viewer is either loading file in background (Esc still closes) or ready with content.
pub enum ViewerState {
    /// File is being read on a background thread; Esc closes without waiting.
    Loading {
        file_path: String,
        rx: mpsc::Receiver<io::Result<Vec<u8>>>,
        /// When set (e.g. from Find file content search), scroll to this 1-based line when ready.
        initial_line: Option<u64>,
    },
    /// Content loaded; normal view.
    Ready(ViewerScreenState),
}

/// State when the file viewer content is ready (F3). Text and hex modes.
pub struct ViewerScreenState {
    pub file_path: String,
    /// Raw file bytes (for hex mode; text mode uses same buffer decoded).
    pub content: Vec<u8>,
    /// Display mode: text (lines) or hex dump.
    pub view_mode: ViewerMode,
    /// First visible line index (scroll offset).
    pub scroll: usize,
    /// Hex mode: byte offset of the current character (highlighted in hex and ASCII columns).
    pub hex_cursor: usize,
    /// Last draw area (for consistent layout).
    pub area: Rect,
    /// Text mode: byte offset of start of each logical line (len = num_lines+1). Used for fast paging (MC-style).
    pub text_line_starts: Option<Vec<usize>>,
    /// Text mode: cumulative display line count after each logical line. cumulative[i] = total display lines for logical lines 0..=i.
    pub text_display_cumulative: Option<Vec<usize>>,
    /// Text mode: content width (chars) this cache was built for; 0 = invalid.
    pub text_cache_width: u16,
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
    /// F3 selection mode (MC-style): when true, arrows extend selection.
    pub selection_extend_mode: bool,
}

#[derive(Debug, Clone)]
pub struct CopyErrorState {
    pub operation: Operation,
    pub message: String,
}

impl AppState {
    /// Create app with initial panel dirs and settings from ~/.oxide/settings.json (used on startup).
    /// If a saved path is missing/invalid, that panel is opened in home_dir.
    /// Panels are created with default state, then sync_from_persisted_settings() is called so
    /// view_mode and show_hidden (and file lists) match persisted_settings before first render.
    pub fn new_with_initial(
        left_cwd: String,
        right_cwd: String,
        home_dir: &str,
        settings: PersistedSettings,
    ) -> io::Result<Self> {
        let left = Panel::new(left_cwd.clone()).or_else(|_| Panel::new(home_dir.to_string()))?;
        let right = Panel::new(right_cwd.clone()).or_else(|_| Panel::new(home_dir.to_string()))?;
        let active = if settings.active_panel == 0 { 0 } else { 1 };
        let mut app = Self {
            active_panel: active,
            left_panel: left,
            right_panel: right,
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
            find_dialog: None,
            settings_dialog: None,
            left_panel_settings_overlay: None,
            right_panel_settings_overlay: None,
            left_panel_rect: None,
            right_panel_rect: None,
            size_info_pending_rx: None,
            find_search_rx: None,
            last_mouse_click: None,
            last_mouse_position: None,
            show_hidden_files: true,
            persisted_settings: settings,
        };
        app.sync_from_persisted_settings();
        Ok(app)
    }

    /// Single sync point: apply persisted_settings to both panels (view_mode, show_hidden, refresh file lists).
    /// Call after startup and whenever persisted_settings change so UI always matches the source of truth.
    pub fn sync_from_persisted_settings(&mut self) {
        let view_left = if self.persisted_settings.left_view.as_str() == "one" {
            ViewMode::SingleColumn
        } else {
            ViewMode::DoubleColumn
        };
        let view_right = if self.persisted_settings.right_view.as_str() == "one" {
            ViewMode::SingleColumn
        } else {
            ViewMode::DoubleColumn
        };
        let left_show = self.persisted_settings.left_show_hidden;
        let right_show = self.persisted_settings.right_show_hidden;
        self.left_panel_mut().set_view_mode(view_left);
        self.right_panel_mut().set_view_mode(view_right);
        self.left_panel_mut().set_show_hidden(left_show);
        self.right_panel_mut().set_show_hidden(right_show);
        let left_sort = self.persisted_settings.left_sort.clone();
        let right_sort = self.persisted_settings.right_sort.clone();
        let left_dirs_first = self.persisted_settings.left_dirs_first;
        let right_dirs_first = self.persisted_settings.right_dirs_first;
        self.left_panel_mut().set_sort_mode(&left_sort);
        self.right_panel_mut().set_sort_mode(&right_sort);
        self.left_panel_mut().set_dirs_first(left_dirs_first);
        self.right_panel_mut().set_dirs_first(right_dirs_first);
        let _ = self.left_panel_mut().refresh_files();
        let _ = self.right_panel_mut().refresh_files();
        self.show_hidden_files = self.active_panel_ref().get_show_hidden();
    }

    /// If autosave is on, write current panel dirs and active panel to persisted_settings and save to file.
    pub fn maybe_persist_panel_dirs(&mut self) {
        if !self.persisted_settings.autosave {
            return;
        }
        let loc_left = self.left_panel.current_location();
        let loc_right = self.right_panel.current_location();
        self.persisted_settings.left_cwd = loc_left.as_fs_path().map(|p| p.to_string_lossy().to_string());
        self.persisted_settings.right_cwd = loc_right.as_fs_path().map(|p| p.to_string_lossy().to_string());
        self.persisted_settings.active_panel = if self.active_panel == 0 { 0 } else { 1 };
        let _ = crate::settings::save(&self.persisted_settings);
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

    /// Open Ctrl+F Find file dialog with start dir from active panel.
    pub fn open_find_dialog(&mut self) {
        let start_dir = self.active_panel_ref().get_current_dir();
        self.find_dialog = Some(FindDialogState {
            phase: FindDialogPhase::Parameter,
            start_dir: start_dir.to_string(),
            start_dir_cursor: start_dir.len(),
            file_pattern: String::new(),
            file_pattern_cursor: 0,
            content_pattern: String::new(),
            content_pattern_cursor: 0,
            recursive: true,
            file_case_sens: false,
            content_case_sens: false,
            skip_hidden: true,
            results: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            status_message: String::new(),
            search_current_dir: String::new(),
            visible_list_rows: 18,
            focus: 0,
        });
        self.find_search_rx = None;
        self.focus = Focus::FindDialog;
    }

    pub fn close_find_dialog(&mut self) {
        self.find_dialog = None;
        self.find_search_rx = None;
        self.focus = Focus::Panel;
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

    /// Current panel location (for backend operations and CopyParams).
    pub fn get_current_location(&self) -> PanelLocation {
        match self.active_panel {
            0 => self.left_panel.current_location().clone(),
            1 => self.right_panel.current_location().clone(),
            _ => self.left_panel.current_location().clone(),
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

    /// Filesystem path to use as copy/move target (for panel_backend). When opposite is Fs, that path; when Zip, the directory containing the archive.
    pub fn get_opposite_panel_target_fs_path(&self) -> PathBuf {
        let loc = match self.active_panel {
            0 => self.right_panel.current_location(),
            1 => self.left_panel.current_location(),
            _ => self.right_panel.current_location(),
        };
        match loc {
            PanelLocation::Fs(p) => p.clone(),
            PanelLocation::Zip { archive, .. } => archive
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("/")),
        }
    }

    /// Sync the process current directory to the active panel's directory.
    /// Only when the active panel is on the filesystem (not inside a ZIP). Call after panel navigation.
    pub fn sync_process_cwd_to_active_panel(&self) {
        let loc = self.get_current_location();
        if let Some(p) = loc.as_fs_path() {
            if let Err(e) = std::env::set_current_dir(p) {
                eprintln!("Failed to change directory: {}", e);
            }
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

    /// Insert a string at the current command-line cursor (e.g. for Ctrl+Enter to insert current file).
    pub fn command_line_insert_str(&mut self, s: &str) {
        let at = self.command_line_cursor.min(self.command_line.len());
        self.command_line.insert_str(at, s);
        self.command_line_cursor = at + s.len();
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
        let target_loc = if new_active_panel == 0 {
            self.left_panel.current_location()
        } else {
            self.right_panel.current_location()
        };
        if let Some(p) = target_loc.as_fs_path() {
            if let Err(e) = std::env::set_current_dir(p) {
                eprintln!("Failed to change directory: {}", e);
            }
        }
        self.active_panel = new_active_panel;
        Ok(())
    }
}
