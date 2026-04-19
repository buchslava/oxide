use crate::browser::diff_viewer::{DiffViewerState, FolderCompareState};
use crate::browser::editor::EditorViewState;
use crate::browser::panel::{Panel, PanelOperations, ViewMode};
use crate::core::settings::{self, PersistedSettings};
use crate::core::trash_delete::trash_available;
use crate::dialogs::error_detail_dialog::ErrorDetailState;
use crate::dialogs::help_dialog::HelpDialogState;
use crate::dialogs::pattern_select_dialog::PatternSelectDialogState;
use crate::ui::theme::{ThemeId, UiPalette};
use crate::ui::toast::{TimedToast, ToastKind};
use ratatui::layout::Rect;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

pub use crate::core::copy_state::{
    ArchiveProgress, CopyErrorState, CopyInProgress, CopyParams, CopyProgress, Operation,
};
use crate::core::location::PanelLocation;
use std::path::PathBuf;

/// After a command with auto-reopen: countdown before restoring the panel TUI.
#[derive(Debug)]
pub struct PostCommandCountdown {
    pub reveal_at: std::time::Instant,
    /// When true, shell output is still on the main buffer; only a small overlay is drawn.
    pub overlay_on_main_buffer: bool,
}

fn view_mode_from_settings_flag(view: &str) -> ViewMode {
    if view == "one" {
        ViewMode::SingleColumn
    } else {
        ViewMode::DoubleColumn
    }
}

/// Message from the background archive thread: progress update or completion.
#[derive(Debug)]
pub enum ArchiveMessage {
    Progress(ArchiveProgress),
    /// On success, the Option is the archive file name (for panel refresh/selection).
    Done(io::Result<()>, Option<String>),
}

/// Single source of truth for input target: panel (navigation), command line (typing), or a modal dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Panel,
    CommandLine,
    /// Find file dialog (Ctrl+F) has focus; panel and command line do not receive keys.
    FindDialog,
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
    /// When Some, the embedded code editor is open (F4): warning, loading, or ready. None = panels view.
    pub editor_screen: Option<EditorViewState>,
    /// When true, show "Save changes?" (1=Save, 2=Discard, 3/Esc=Cancel) before exiting editor.
    pub editor_confirm_pending: bool,
    /// Editor confirm dialog: focused option index 0=Save, 1=Discard, 2=Cancel.
    pub editor_confirm_focus: usize,
    /// Short-lived ratatui toast (e.g. editor save); see [`crate::ui::toast::TimedToast`].
    pub timed_toast: Option<TimedToast>,
    /// Command finished with auto-reopen: countdown before restoring panel UI.
    pub post_command_countdown: Option<PostCommandCountdown>,
    /// When Some, the file viewer is open (F3). Loading = reading file in background; Ready = content available. None = panels or editor view.
    pub viewer_screen: Option<ViewerState>,
    /// When Some, Ctrl+D two-file diff viewer is open (full screen, synchronized scroll).
    pub diff_viewer_screen: Option<DiffViewerState>,
    /// When Some, Ctrl+D compared both panel directories: `C `/`S `/`X ` prefixes until either cwd changes.
    pub folder_compare: Option<FolderCompareState>,
    /// When Some, F7 "Create directory" dialog is open (text field for new folder name).
    pub mkdir_dialog: Option<MkdirDialogState>,
    /// When Some, Ctrl+A "Archive" dialog is open (text field for archive file name).
    pub archive_dialog: Option<ArchiveDialogState>,
    /// When Some, Ctrl+N "New file" dialog is open (text field for new file name).
    pub new_file_dialog: Option<NewFileDialogState>,
    /// When Some, show error message after new file dialog (e.g. file already exists).
    pub new_file_error: Option<String>,
    /// When Some, archiving is in progress; show progress overlay (like copy progress).
    pub archive_progress: Option<ArchiveProgress>,
    /// Receiver for background archive thread; polled in main loop.
    pub archive_pending_rx: Option<mpsc::Receiver<ArchiveMessage>>,
    /// When Some, the archive thread should stop; set on ESC during archive progress.
    pub archive_cancel: Option<Arc<AtomicBool>>,
    /// When Some, F2 "Rename / Attributes" dialog is open (single file: name + attrs; group: attrs only).
    pub rename_attr_dialog: Option<RenameAttrDialogState>,
    /// When Some, an error alert is shown on top of the F2 dialog (message to display).
    pub rename_attr_error: Option<String>,
    /// When Some, Ctrl+X then S "Size info" dialog is open (total size of selected items).
    pub size_info_dialog: Option<SizeInfoDialogState>,
    /// When Some, Ctrl+F "Find file" dialog is open.
    pub find_dialog: Option<FindDialogState>,
    /// When Some, + / − mark or unmark by file glob (same as Find file pattern).
    pub pattern_select_dialog: Option<PatternSelectDialogState>,
    /// When Some, F9 Settings dialog is open (two-column: sections list + content).
    pub settings_dialog: Option<SettingsDialogState>,
    /// When Some, F1 Help dialog is open (scroll position in state).
    pub help_dialog: Option<HelpDialogState>,
    /// When Some, scrollable error details (invalid zip, navigation I/O, etc.).
    pub error_detail: Option<ErrorDetailState>,
    /// When Some, Ctrl+X then 1 "Left panel settings" overlay is open over the left panel.
    pub left_panel_settings_overlay: Option<PanelSettingsOverlayState>,
    /// When Some, Ctrl+X then 2 "Right panel settings" overlay is open over the right panel.
    pub right_panel_settings_overlay: Option<PanelSettingsOverlayState>,
    /// Last frame's left panel area (set by UI renderer); used to position left panel overlay.
    pub left_panel_rect: Option<Rect>,
    /// Last frame's right panel area; used to position right panel overlay.
    pub right_panel_rect: Option<Rect>,
    /// Receiver for background size calculation; polled in main loop.
    pub size_info_pending_rx: Option<mpsc::Receiver<SizeInfoProgress>>,
    /// Receiver for find file search thread; polled when find_dialog is open.
    pub find_search_rx: Option<mpsc::Receiver<FindMessage>>,
    /// When Some, the find search thread should stop; set on Esc/close during search.
    pub find_search_cancel: Option<Arc<AtomicBool>>,
    /// Last left-click (instant, panel_index, file_index) for double-click detection.
    pub last_mouse_click: Option<(std::time::Instant, usize, usize)>,
    /// Last mouse (column, row) from any mouse event (scroll, move, click).
    pub last_mouse_position: Option<(u16, u16)>,
    /// Last file-name pattern from Find (Ctrl+F) and +/−; pre-filled when reopening those dialogs.
    pub last_file_name_pattern: String,
    /// When true, show hidden files (names starting with "."). Default true. Toggled by Ctrl+H.
    pub show_hidden_files: bool,
    /// Last saved/loaded settings from ~/.oxide/settings.json. Used to persist on change and for autosave.
    pub persisted_settings: PersistedSettings,
    /// At startup: OS trash usable for Safe delete (Linux: writable XDG data dir; macOS: yes).
    pub trash_available: bool,
    /// Active built-in theme id; kept in sync with `persisted_settings.theme` and `ui_palette`.
    pub theme_id: ThemeId,
    /// Resolved colors for this frame; update when `theme_id` changes via [`ThemeId::palette`].
    pub ui_palette: UiPalette,
    /// After **Ctrl+X**, the next key completes an Oxide shortcut (e.g. `F` for find, `O` for shell).
    pub ctrl_x_chord_pending: bool,
}

pub use crate::dialogs::panel_overlay_state::PanelSettingsOverlayState;
pub use crate::dialogs::size_info_dialog::{SizeInfoDialogState, SizeInfoProgress};

// Re-exports so AppState and other modules can use these types without circular deps.
pub use crate::browser::viewer::ViewerState;
pub use crate::core::find::FindMessage;
pub use crate::dialogs::archive_dialog::ArchiveDialogState;
pub use crate::dialogs::find_dialog::{FindDialogPhase, FindDialogState};
pub use crate::dialogs::mkdir_dialog::MkdirDialogState;
pub use crate::dialogs::new_file_dialog::NewFileDialogState;
pub use crate::dialogs::rename_attr::{RenameAttrDialogState, RenameAttrField};
pub use crate::dialogs::settings_dialog::SettingsDialogState;

impl AppState {
    /// Store trimmed pattern for reuse in Find and +/− (empty clears remembered pattern).
    pub fn set_last_file_name_pattern(
        &mut self,
        raw: &str,
    ) {
        self.last_file_name_pattern = raw.trim().to_string();
    }

    /// Create app with initial panel dirs and settings from ~/.oxide/settings.json (used on startup).
    /// Path resolution is handled in `main` (autosave vs launch dir vs saved opposite panel). If a
    /// path is invalid, that panel falls back to `home_dir`.
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
            timed_toast: None,
            post_command_countdown: None,
            viewer_screen: None,
            diff_viewer_screen: None,
            folder_compare: None,
            mkdir_dialog: None,
            archive_dialog: None,
            new_file_dialog: None,
            new_file_error: None,
            archive_progress: None,
            archive_pending_rx: None,
            archive_cancel: None,
            rename_attr_dialog: None,
            rename_attr_error: None,
            size_info_dialog: None,
            find_dialog: None,
            pattern_select_dialog: None,
            settings_dialog: None,
            help_dialog: None,
            error_detail: None,
            left_panel_settings_overlay: None,
            right_panel_settings_overlay: None,
            left_panel_rect: None,
            right_panel_rect: None,
            size_info_pending_rx: None,
            find_search_rx: None,
            find_search_cancel: None,
            last_mouse_click: None,
            last_mouse_position: None,
            last_file_name_pattern: String::new(),
            show_hidden_files: true,
            persisted_settings: settings,
            trash_available: trash_available(),
            theme_id: ThemeId::default(),
            ui_palette: ThemeId::default().palette(),
            ctrl_x_chord_pending: false,
        };
        app.sync_from_persisted_settings();
        Ok(app)
    }

    /// Single sync point: apply persisted_settings to both panels (view_mode, show_hidden, refresh file lists).
    /// Call after startup and whenever persisted_settings change so UI always matches the source of truth.
    pub fn sync_from_persisted_settings(&mut self) {
        self.theme_id = ThemeId::from_slug(&self.persisted_settings.theme);
        self.ui_palette = self.theme_id.palette();
        let view_left = view_mode_from_settings_flag(&self.persisted_settings.left_view);
        let view_right = view_mode_from_settings_flag(&self.persisted_settings.right_view);
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

    /// Write current left/right paths and active panel to `persisted_settings` and `settings.json`
    /// (same data as **Autosave latest state** when it saves).
    pub fn persist_panel_state_to_settings(&mut self) -> io::Result<()> {
        let loc_left = self.left_panel.current_location();
        let loc_right = self.right_panel.current_location();
        self.persisted_settings.left_cwd = loc_left
            .as_fs_path()
            .map(|p| p.to_string_lossy().to_string());
        self.persisted_settings.right_cwd = loc_right
            .as_fs_path()
            .map(|p| p.to_string_lossy().to_string());
        self.persisted_settings.active_panel = if self.active_panel == 0 { 0 } else { 1 };
        settings::save(&self.persisted_settings)
    }

    /// If autosave is on, write current panel dirs and active panel to persisted_settings and save to file.
    pub fn maybe_persist_panel_dirs(&mut self) {
        if !self.persisted_settings.autosave {
            return;
        }
        let _ = self.persist_panel_state_to_settings();
    }

    /// True while the post-command countdown is running (main-buffer overlay or waiting to restore TUI).
    pub fn post_command_countdown_active(&self) -> bool {
        self.post_command_countdown
            .as_ref()
            .is_some_and(|c| std::time::Instant::now() < c.reveal_at)
    }

    pub fn set_timed_toast(
        &mut self,
        duration: Duration,
        message: impl Into<String>,
    ) {
        self.set_timed_toast_kind(duration, message, ToastKind::Info);
    }

    /// Same as [`Self::set_timed_toast`] but with a red alert background (warnings / errors).
    pub fn set_timed_toast_alert(
        &mut self,
        duration: Duration,
        message: impl Into<String>,
    ) {
        self.set_timed_toast_kind(duration, message, ToastKind::Alert);
    }

    fn set_timed_toast_kind(
        &mut self,
        duration: Duration,
        message: impl Into<String>,
        kind: ToastKind,
    ) {
        self.timed_toast = Some(TimedToast::with_kind(
            duration,
            message.into(),
            kind,
        ));
    }

    pub fn clear_timed_toast(&mut self) {
        self.timed_toast = None;
    }

    /// Countdown is drawn on the main buffer over shell output; skip ratatui `draw` until it ends.
    pub fn post_command_countdown_on_main_buffer(&self) -> bool {
        self.post_command_countdown
            .as_ref()
            .is_some_and(|c| c.overlay_on_main_buffer && std::time::Instant::now() < c.reveal_at)
    }

    /// Set the active panel by index (0 = left, 1 = right).
    pub fn set_active_panel(
        &mut self,
        panel_index: usize,
    ) {
        self.active_panel = if panel_index == 0 { 0 } else { 1 };
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

    /// Location of the panel opposite to the active one (target for F5 Copy / F6 Move).
    pub fn get_opposite_panel_location(&self) -> PanelLocation {
        match self.active_panel {
            0 => self.right_panel.current_location().clone(),
            1 => self.left_panel.current_location().clone(),
            _ => self.right_panel.current_location().clone(),
        }
    }

    /// Filesystem path to use as copy/move target when opposite is Fs. When opposite is Zip, use target_location and copy into archive instead.
    pub fn get_opposite_panel_target_fs_path(&self) -> PathBuf {
        let loc = self.get_opposite_panel_location();
        match &loc {
            PanelLocation::Fs(p) => p.clone(),
            PanelLocation::Archive { archive, .. } => archive
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("/")),
        }
    }

    /// Set the process working directory to the **active** panel's filesystem path (`active_panel`:
    /// 0 = left, 1 = right). No-op if that panel is inside a ZIP.
    fn apply_process_cwd_to_active_panel_fs(&self) {
        let loc = self.get_current_location();
        if let Some(p) = loc.as_fs_path() {
            if let Err(e) = std::env::set_current_dir(p) {
                eprintln!("Failed to change directory: {}", e);
            }
        }
    }

    /// When **Autosave latest state** is off, keep the terminal/process `cwd` aligned with the
    /// **active** panel's directory (left vs right follows [`Self::active_panel`]). When autosave is
    /// on, directories are stored in settings instead and the process `cwd` is not updated on navigation.
    pub fn sync_process_cwd_to_active_panel_if_no_autosave(&self) {
        if self.persisted_settings.autosave {
            return;
        }
        self.apply_process_cwd_to_active_panel_fs();
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

    pub fn command_line_insert(
        &mut self,
        c: char,
    ) {
        let at = self.command_line_cursor.min(self.command_line.len());
        self.command_line.insert(at, c);
        self.command_line_cursor = at + 1;
    }

    /// Insert a string at the current command-line cursor (e.g. for Ctrl+Enter to insert current file).
    pub fn command_line_insert_str(
        &mut self,
        s: &str,
    ) {
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

    /// Remove the character after the cursor (forward delete). Cursor is a byte offset into `command_line`.
    pub fn command_line_delete_forward(&mut self) {
        if self.command_line_cursor >= self.command_line.len() {
            return;
        }
        let tail = &self.command_line[self.command_line_cursor..];
        let Some(ch) = tail.chars().next() else {
            return;
        };
        let n = ch.len_utf8();
        self.command_line
            .drain(self.command_line_cursor..self.command_line_cursor + n);
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
        self.active_panel = new_active_panel;
        self.sync_process_cwd_to_active_panel_if_no_autosave();
        Ok(())
    }
}
