use std::collections::HashSet;
use std::fs::metadata;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::file_ops::FileInfo;
use crate::core::find::PreparedFilePattern;
use crate::core::location::PanelLocation;
use crate::core::panel_backend;

/// Index of a non–parent-dir entry whose name matches `name` (trimmed trailing `/`).
fn fs_path_is_listable(path: &Path) -> bool {
    metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

/// Walk up from `path` until a listable directory is found (handles deleted cwd).
fn climb_to_valid_fs_path(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    loop {
        if fs_path_is_listable(&p) {
            return p;
        }
        match p.parent() {
            None => return PathBuf::from("/"),
            Some(parent) if parent.as_os_str().is_empty() => return PathBuf::from("/"),
            Some(parent) => {
                if parent == p {
                    return PathBuf::from("/");
                }
                p = parent.to_path_buf();
            }
        }
    }
}

fn index_of_non_parent_file_named(
    files: &[FileInfo],
    name: &str,
) -> Option<usize> {
    let name_trimmed = name.trim_end_matches('/');
    files
        .iter()
        .position(|f| !f.is_parent_dir() && f.name.trim_end_matches('/') == name_trimmed)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewMode {
    SingleColumn,
    DoubleColumn,
}

pub trait PanelOperations {
    fn move_up(
        &mut self,
        panel_height: usize,
    );
    fn move_down(
        &mut self,
        panel_height: usize,
    );
    fn page_up(
        &mut self,
        panel_height: usize,
    );
    fn page_down(
        &mut self,
        panel_height: usize,
    );
    fn smart_move_left(
        &mut self,
        panel_height: usize,
    );
    fn smart_move_right(
        &mut self,
        panel_height: usize,
    );
    fn enter_directory(&mut self) -> io::Result<()>;
    fn refresh_files(&mut self) -> io::Result<()>;
    fn update_scroll_offset(
        &mut self,
        panel_height: usize,
    );
    fn update_scroll_offset_double_column(
        &mut self,
        panel_height: usize,
    );
    fn get_current_dir(&self) -> &str;
    fn get_selected_file(&self) -> Option<&FileInfo>;
    fn get_selected_index(&self) -> usize;
    fn get_scroll_offset(&self) -> usize;
    fn get_files(&self) -> &[FileInfo];
    fn get_view_mode(&self) -> ViewMode;
    fn set_view_mode(
        &mut self,
        mode: ViewMode,
    );
    /// Toggle selection (mark) of the current file and move to the next. F12 / MC Insert.
    fn toggle_mark_and_move_next(
        &mut self,
        panel_height: usize,
    );
    /// Invert selection: all marked become unmarked, all unmarked (except "..") become marked. MC *.
    fn invert_selection(&mut self);
    /// Mark every non–parent-dir entry whose name matches the pattern (wildcards or regex per F9 Settings).
    fn mark_matching_glob(
        &mut self,
        pattern: &str,
        case_sensitive: bool,
        use_regex: bool,
    );
    /// Unmark entries matching the pattern. The file under the cursor keeps its mark so a broad pattern
    /// does not clear the “current” tagged row.
    fn unmark_matching_glob(
        &mut self,
        pattern: &str,
        case_sensitive: bool,
        use_regex: bool,
    );
    fn is_marked(
        &self,
        index: usize,
    ) -> bool;
    /// Same as get_names_to_copy plus names of file before (first-1) and after (first+count) for restore after delete/move.
    fn get_names_to_copy_with_restore_neighbors(
        &self
    ) -> (
        Vec<(String, bool)>,
        Option<String>,
        Option<String>,
    );
}

#[derive(Debug)]
pub struct Panel {
    pub view_mode: ViewMode,
    current_location: PanelLocation,
    /// Cached display string for get_current_dir() (kept in sync with current_location).
    current_dir_display: String,
    files: Vec<FileInfo>,
    selected_index: usize,
    scroll_offset: usize,
    navigation_history: Vec<(PanelLocation, usize)>,
    /// Indices of files marked for group operations (F12 / MC Insert).
    marked_indices: HashSet<usize>,
    /// When true, show hidden files (names starting with "."). Toggled by Ctrl+H.
    show_hidden: bool,
    /// Sort mode: name_asc, name_desc, size_asc, size_desc, mtime_asc, mtime_desc.
    sort_mode: String,
    /// When true (default), directories appear before files; when false, unified sort.
    dirs_first: bool,
}

impl Panel {
    pub fn new(dir: String) -> io::Result<Self> {
        let loc = PanelLocation::fs(dir);
        let current_dir_display = loc.display_string();
        let mut panel = Self {
            view_mode: ViewMode::DoubleColumn,
            current_location: loc,
            current_dir_display,
            files: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            navigation_history: Vec::new(),
            marked_indices: HashSet::new(),
            show_hidden: true,
            sort_mode: "name_asc".to_string(),
            dirs_first: true,
        };
        panel.refresh_files()?;
        Ok(panel)
    }

    /// Navigate to a new location (e.g. from Find file "Chdir"). Public for use from main.
    pub fn navigate_to_location(
        &mut self,
        new_location: PanelLocation,
    ) -> io::Result<()> {
        self.marked_indices.clear();
        self.navigation_history.push((
            self.current_location.clone(),
            self.selected_index,
        ));
        self.current_location = new_location;
        self.current_dir_display = self.current_location.display_string();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.refresh_files()
    }

    fn navigate_to_parent(&mut self) -> io::Result<()> {
        let Some(parent) = self.current_location.parent() else {
            return Ok(());
        };
        self.marked_indices.clear();
        self.navigation_history.push((
            self.current_location.clone(),
            self.selected_index,
        ));
        self.current_location = parent.clone();
        self.current_dir_display = self.current_location.display_string();
        self.scroll_offset = 0;
        self.refresh_files()?;

        // Try to find the directory we came from in the parent list
        if let Some((prev_loc, _)) = self.navigation_history.pop() {
            let prev_name_str = match &prev_loc {
                PanelLocation::Fs(p) => p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string()),
                PanelLocation::Archive {
                    archive,
                    path_inside,
                    ..
                } => {
                    let inside = path_inside.trim_end_matches('/');
                    if inside.is_empty() {
                        archive
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|s| s.to_string())
                    } else {
                        inside
                            .rsplit_once('/')
                            .map(|(_, last)| last.to_string())
                            .or_else(|| Some(inside.to_string()))
                    }
                }
            };
            if let Some(prev_name_str) = prev_name_str {
                for (i, file) in self.files.iter().enumerate() {
                    if !file.is_parent_dir() {
                        let file_name_clean = file.name.trim_end_matches('/');
                        if file_name_clean == prev_name_str {
                            self.selected_index = i;
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// If the current path no longer exists (e.g. directory deleted), move to the nearest listable parent.
    fn climb_to_existing_path(&mut self) {
        match &self.current_location {
            PanelLocation::Fs(path) => {
                if !fs_path_is_listable(path) {
                    let new_path = climb_to_valid_fs_path(path);
                    self.current_location = PanelLocation::fs(new_path);
                    self.current_dir_display = self.current_location.display_string();
                }
            }
            PanelLocation::Archive { archive, .. } => {
                if !archive.exists() {
                    self.current_location = archive
                        .parent()
                        .map(|p| PanelLocation::fs(p.to_path_buf()))
                        .unwrap_or_else(|| PanelLocation::fs("/"));
                    self.current_dir_display = self.current_location.display_string();
                }
            }
        }
    }

    fn try_list_current_location(&mut self) -> io::Result<Vec<FileInfo>> {
        match panel_backend::list(
            &self.current_location,
            self.show_hidden,
            &self.sort_mode,
            self.dirs_first,
        ) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.climb_to_existing_path();
                panel_backend::list(
                    &self.current_location,
                    self.show_hidden,
                    &self.sort_mode,
                    self.dirs_first,
                )
            }
            other => other,
        }
    }

    /// Reference to current location (for backend operations).
    pub fn current_location(&self) -> &PanelLocation {
        &self.current_location
    }
}

impl PanelOperations for Panel {
    fn move_up(
        &mut self,
        panel_height: usize,
    ) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            if self.view_mode == ViewMode::DoubleColumn {
                let h = panel_height.max(1);
                let files_per_page = h * 2;
                // MC scroll_pages: when cursor went above visible area, scroll up by half page
                if self.selected_index < self.scroll_offset {
                    self.scroll_offset = self.scroll_offset.saturating_sub(files_per_page / 2);
                }
                self.update_scroll_offset_double_column(panel_height);
            } else {
                self.update_scroll_offset(panel_height);
            }
        }
    }

    fn move_down(
        &mut self,
        panel_height: usize,
    ) {
        if !self.files.is_empty() && self.selected_index < self.files.len() - 1 {
            self.selected_index += 1;
            if self.view_mode == ViewMode::DoubleColumn {
                let h = panel_height.max(1);
                let files_per_page = h * 2;
                // MC scroll_pages: when cursor hits bottom of visible area, scroll down by half page
                if self.selected_index == self.scroll_offset + files_per_page {
                    let max_scroll = self.files.len().saturating_sub(files_per_page).max(0);
                    self.scroll_offset = (self.scroll_offset + files_per_page / 2).min(max_scroll);
                }
                self.update_scroll_offset_double_column(panel_height);
            } else {
                self.update_scroll_offset(panel_height);
            }
        }
    }

    fn page_up(
        &mut self,
        panel_height: usize,
    ) {
        let h = panel_height.max(1);
        if self.view_mode == ViewMode::DoubleColumn {
            // MC prev_page: move current and top together by one page (or less if near top)
            let files_per_page = h * 2;
            if self.files.is_empty() || (self.selected_index == 0 && self.scroll_offset == 0) {
                return;
            }
            let items = files_per_page.min(self.scroll_offset);
            if items == 0 {
                self.selected_index = 0;
            } else {
                self.selected_index = self.selected_index.saturating_sub(items);
                self.scroll_offset -= items;
            }
            self.update_scroll_offset_double_column(panel_height);
        } else {
            if self.selected_index < h {
                self.selected_index = 0;
            } else {
                self.selected_index -= h;
            }
            self.update_scroll_offset(panel_height);
        }
    }

    fn page_down(
        &mut self,
        panel_height: usize,
    ) {
        let h = panel_height.max(1);
        if self.files.is_empty() {
            return;
        }
        let total = self.files.len();
        if self.view_mode == ViewMode::DoubleColumn {
            // MC next_page: current += items, top += items; items = min(files_per_page, room at end)
            let files_per_page = h * 2;
            let max_scroll = total.saturating_sub(files_per_page).max(0);
            if self.selected_index >= total - 1 {
                return;
            }
            let mut items = files_per_page;
            if self.scroll_offset > max_scroll.saturating_sub(files_per_page) {
                items = total
                    .saturating_sub(files_per_page)
                    .saturating_sub(self.scroll_offset);
            }
            if self.scroll_offset + items > max_scroll {
                items = max_scroll.saturating_sub(self.scroll_offset);
            }
            if items == 0 {
                self.selected_index = total - 1;
            } else {
                self.selected_index = (self.selected_index + items).min(total - 1);
                self.scroll_offset += items;
            }
            self.update_scroll_offset_double_column(panel_height);
        } else {
            if self.selected_index >= total - 1 {
                return;
            }
            if self.selected_index + h >= total {
                self.selected_index = total - 1;
            } else {
                self.selected_index += h;
            }
            self.update_scroll_offset(panel_height);
        }
    }

    /// MC move_left: panel_move_current(panel, -panel_lines). Same as MC: current -= lines;
    /// if current goes above visible window, top += lines (top moves up by one column).
    fn smart_move_left(
        &mut self,
        panel_height: usize,
    ) {
        let h = panel_height.max(1);
        let files_per_page = h * 2;
        let total = self.files.len();
        if total == 0 || self.selected_index >= total {
            return;
        }
        let lines = h;
        let new_pos = self.selected_index.saturating_sub(lines);
        self.selected_index = new_pos.min(total - 1);
        let mut adjust = false;
        if self.selected_index >= self.scroll_offset + files_per_page {
            self.scroll_offset = self.scroll_offset.saturating_add(lines);
            adjust = true;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.scroll_offset.saturating_sub(lines);
            adjust = true;
        }
        if adjust {
            self.scroll_offset = self.scroll_offset.min(self.selected_index);
            self.scroll_offset = self.scroll_offset.max(0);
            let max_scroll = total.saturating_sub(files_per_page).max(0);
            self.scroll_offset = self.scroll_offset.min(max_scroll);
        }
        self.update_scroll_offset_double_column(panel_height);
    }

    /// MC move_right: panel_move_current(panel, panel_lines). Same as MC: current += lines;
    /// if current goes below visible window, top += lines (window scrolls down by one column).
    fn smart_move_right(
        &mut self,
        panel_height: usize,
    ) {
        let h = panel_height.max(1);
        let files_per_page = h * 2;
        let total = self.files.len();
        if total == 0 || self.selected_index >= total.saturating_sub(1) {
            return;
        }
        let lines = h;
        let new_pos = (self.selected_index + lines).min(total - 1);
        self.selected_index = new_pos;
        let mut adjust = false;
        if self.selected_index >= self.scroll_offset + files_per_page {
            self.scroll_offset += lines;
            adjust = true;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.scroll_offset.saturating_sub(lines);
            adjust = true;
        }
        if adjust {
            self.scroll_offset = self.scroll_offset.min(self.selected_index);
            self.scroll_offset = self.scroll_offset.max(0);
            let max_scroll = total.saturating_sub(files_per_page).max(0);
            self.scroll_offset = self.scroll_offset.min(max_scroll);
        }
        self.update_scroll_offset_double_column(panel_height);
    }

    fn enter_directory(&mut self) -> io::Result<()> {
        if let Some(file) = self.get_selected_file() {
            if file.is_parent_dir() {
                self.navigate_to_parent()?;
                return Ok(());
            }
            if file.is_dir {
                if let Some(new_loc) = self.current_location.enter(&file.name, true) {
                    self.navigate_to_location(new_loc)?;
                }
                return Ok(());
            }
            // Enter on a file: supported archive extensions (open as virtual folder)
            let name_clean = file.name.trim_end_matches('/');
            let lower = name_clean.to_lowercase();
            if lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
                if let Some(new_loc) = self.current_location.enter(name_clean, false) {
                    self.navigate_to_location(new_loc)?;
                }
            }
        }
        Ok(())
    }

    /// Re-read the current directory from disk. **Resets** `selected_index`, `scroll_offset`, and all marks.
    /// Prefer [`Panel::refresh_files_restore_selection`] when the listing may be unchanged except for
    /// adds/removes and you need to keep the highlighted file and behavior consistent with other code paths.
    fn refresh_files(&mut self) -> io::Result<()> {
        self.climb_to_existing_path();
        self.marked_indices.clear();
        self.files = self.try_list_current_location()?;
        self.selected_index = 0;
        self.scroll_offset = 0;
        if !self.files.is_empty() && self.selected_index >= self.files.len() {
            self.selected_index = self.files.len() - 1;
        }
        Ok(())
    }

    fn update_scroll_offset(
        &mut self,
        panel_height: usize,
    ) {
        let h = panel_height.max(1);
        if self.selected_index >= self.scroll_offset + h {
            self.scroll_offset = self.selected_index - h + 1;
        } else if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }
    }

    /// MC adjust_top_file: keep scroll so current is visible; minimal adjustment.
    /// top in [current - items + 1, current] and in [0, len - items]. Only adjust when
    /// current is outside the visible window (or clamp to valid range).
    fn update_scroll_offset_double_column(
        &mut self,
        panel_height: usize,
    ) {
        let h = panel_height.max(1);
        let files_per_page = h * 2;
        let total_files = self.files.len();

        if total_files <= files_per_page {
            self.scroll_offset = 0;
            return;
        }
        self.selected_index = self.selected_index.min(total_files - 1);

        let max_top = total_files.saturating_sub(files_per_page).max(0);
        if self.scroll_offset > max_top {
            self.scroll_offset = max_top;
        }
        // Selection above visible window: scroll up so current is at top
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }
        // Selection below visible window: scroll down so current is visible
        let min_top = self.selected_index.saturating_sub(files_per_page - 1);
        if self.scroll_offset < min_top {
            self.scroll_offset = min_top.min(max_top);
        }
        if self.scroll_offset > self.selected_index {
            self.scroll_offset = self.selected_index;
        }
        self.scroll_offset = self.scroll_offset.max(0);
    }

    fn get_current_dir(&self) -> &str {
        &self.current_dir_display
    }

    fn get_selected_file(&self) -> Option<&FileInfo> {
        self.files.get(self.selected_index)
    }

    fn get_selected_index(&self) -> usize {
        self.selected_index
    }

    fn get_scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    fn get_files(&self) -> &[FileInfo] {
        &self.files
    }

    fn get_view_mode(&self) -> ViewMode {
        self.view_mode
    }

    fn set_view_mode(
        &mut self,
        mode: ViewMode,
    ) {
        self.view_mode = mode;
    }

    fn toggle_mark_and_move_next(
        &mut self,
        panel_height: usize,
    ) {
        if self.files.is_empty() {
            return;
        }
        let idx = self.selected_index.min(self.files.len() - 1);
        if let Some(f) = self.files.get(idx) {
            if !f.is_parent_dir() {
                if self.marked_indices.contains(&idx) {
                    self.marked_indices.remove(&idx);
                } else {
                    self.marked_indices.insert(idx);
                }
            }
        }
        self.move_down(panel_height);
    }

    fn invert_selection(&mut self) {
        let indices_to_toggle: Vec<usize> = self
            .get_files()
            .iter()
            .enumerate()
            .filter(|(_, file)| !file.is_parent_dir())
            .map(|(idx, _)| idx)
            .collect();
        for idx in indices_to_toggle {
            if self.marked_indices.contains(&idx) {
                self.marked_indices.remove(&idx);
            } else {
                self.marked_indices.insert(idx);
            }
        }
    }

    fn mark_matching_glob(
        &mut self,
        pattern: &str,
        case_sensitive: bool,
        use_regex: bool,
    ) {
        let prep = PreparedFilePattern::new(pattern, case_sensitive, use_regex);
        for (idx, file) in self.files.iter().enumerate() {
            if file.is_parent_dir() {
                continue;
            }
            let name = file.name.trim_end_matches('/');
            if prep.matches(name) {
                self.marked_indices.insert(idx);
            }
        }
    }

    fn unmark_matching_glob(
        &mut self,
        pattern: &str,
        case_sensitive: bool,
        use_regex: bool,
    ) {
        let cursor = self.selected_index;
        let prep = PreparedFilePattern::new(pattern, case_sensitive, use_regex);
        for (idx, file) in self.files.iter().enumerate() {
            if file.is_parent_dir() {
                continue;
            }
            if idx == cursor {
                // Keep the mark on the current row: broad patterns should not "clear" the file under the cursor.
                continue;
            }
            let name = file.name.trim_end_matches('/');
            if prep.matches(name) {
                self.marked_indices.remove(&idx);
            }
        }
    }

    fn is_marked(
        &self,
        index: usize,
    ) -> bool {
        self.marked_indices.contains(&index)
    }

    fn get_names_to_copy_with_restore_neighbors(
        &self
    ) -> (
        Vec<(String, bool)>,
        Option<String>,
        Option<String>,
    ) {
        let files = self.get_files();
        if files.is_empty() {
            return (Vec::new(), None, None);
        }
        let (first_index, last_index, items) = if self.marked_indices.is_empty() {
            if let Some(f) = self.get_selected_file() {
                if !f.is_parent_dir() {
                    let idx = self.selected_index;
                    let mut v = Vec::with_capacity(1);
                    v.push((f.name.clone(), f.is_dir));
                    (idx, idx, v)
                } else {
                    return (Vec::new(), None, None);
                }
            } else {
                return (Vec::new(), None, None);
            }
        } else {
            let first_index = *self
                .marked_indices
                .iter()
                .min()
                .expect("marked_indices non-empty in get_names_to_copy");
            let last_index = *self
                .marked_indices
                .iter()
                .max()
                .expect("marked_indices non-empty in get_names_to_copy");
            let mut items = Vec::new();
            for &idx in &self.marked_indices {
                if let Some(f) = files.get(idx) {
                    if !f.is_parent_dir() {
                        items.push((f.name.clone(), f.is_dir));
                    }
                }
            }
            (first_index, last_index, items)
        };
        if items.is_empty() {
            return (Vec::new(), None, None);
        }
        let name_before = if first_index > 0 {
            let file_before = &files[first_index - 1];
            if !file_before.is_parent_dir() {
                Some(file_before.name.clone())
            } else {
                None
            }
        } else {
            None
        };
        // Prefer the file immediately after the last deleted item (or after the block for single selection).
        let after_index = last_index + 1;
        let name_after = if after_index < files.len() {
            let file_after = &files[after_index];
            if !file_after.is_parent_dir() {
                Some(file_after.name.clone())
            } else {
                None
            }
        } else {
            None
        };
        (items, name_after, name_before)
    }
}

impl Panel {
    /// Clear all marks (Space). Used before Ctrl+D panel-directory compare.
    pub fn clear_marks(&mut self) {
        self.marked_indices.clear();
    }

    /// Yields each marked file index once. Iteration order is unspecified; sort by index if you need list order.
    pub fn iter_marked_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.marked_indices.iter().copied()
    }

    /// Whether hidden files are shown (for Ctrl+H and settings sync).
    pub fn get_show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Set whether hidden files (names starting with ".") are shown. Used by Ctrl+H toggle.
    pub fn set_show_hidden(
        &mut self,
        show: bool,
    ) {
        self.show_hidden = show;
    }

    /// Set sort mode for file list.
    pub fn set_sort_mode(
        &mut self,
        mode: &str,
    ) {
        self.sort_mode = mode.to_string();
    }

    /// Set whether directories appear before files (true) or unified sort (false).
    pub fn set_dirs_first(
        &mut self,
        dirs_first: bool,
    ) {
        self.dirs_first = dirs_first;
    }

    /// Set the current selection to the given index and update scroll so it is visible.
    pub fn set_selection(
        &mut self,
        index: usize,
        panel_height: usize,
    ) {
        let len = self.files.len();
        if len == 0 {
            return;
        }
        self.selected_index = index.min(len - 1);
        match self.view_mode {
            ViewMode::SingleColumn => self.update_scroll_offset(panel_height),
            ViewMode::DoubleColumn => self.update_scroll_offset_double_column(panel_height),
        }
    }

    /// Same directory listing: restore the linear cursor index captured **before** an in-place change
    /// (marks only — `files` order and length are unchanged), clamp to the list, then sync scroll with
    /// the same `panel_height` as keyboard navigation ([`crate::util::compute_panel_height`]).
    ///
    /// Name-based re-anchoring is not used here: display names, zip vs FS, and Unicode can diverge from
    /// `FileInfo::name` and would send the cursor to the wrong row (e.g. `..` at index 0).
    pub fn restore_cursor_after_same_dir_op(
        &mut self,
        saved_index: usize,
        panel_height: usize,
    ) {
        if self.files.is_empty() {
            return;
        }
        self.selected_index = saved_index.min(self.files.len() - 1);
        match self.view_mode {
            ViewMode::SingleColumn => self.update_scroll_offset(panel_height),
            ViewMode::DoubleColumn => self.update_scroll_offset_double_column(panel_height),
        }
    }

    /// Re-read directory and try to restore selection by file name (e.g. after delete/move or running a command).
    /// preferred_after: try to select this file first (file after deleted block).
    /// preferred_before: if preferred_after not found, try this (file before deleted block).
    /// panel_height: if Some, update scroll so selection is visible (pagination).
    pub fn refresh_files_restore_selection(
        &mut self,
        preferred_after: Option<&str>,
        preferred_before: Option<&str>,
        panel_height: Option<usize>,
    ) -> io::Result<()> {
        let prev_selected_index = self.selected_index;
        let prev_selected_name = self
            .files
            .get(self.selected_index)
            .filter(|f| !f.is_parent_dir())
            .map(|f| f.name.trim_end_matches('/').to_string());

        self.marked_indices.clear();
        self.climb_to_existing_path();
        self.files = self.try_list_current_location()?;
        self.selected_index = 0;
        self.scroll_offset = 0;

        let mut found = false;
        if let Some(name) = preferred_after {
            if let Some(i) = index_of_non_parent_file_named(&self.files, name) {
                self.selected_index = i;
                found = true;
            }
        }
        if !found {
            if let Some(name) = preferred_before {
                if let Some(i) = index_of_non_parent_file_named(&self.files, name) {
                    self.selected_index = i;
                    found = true;
                }
            }
        }
        // If no explicit target from delete/move, keep the current file when it still exists.
        if !found {
            if let Some(prev_name) = prev_selected_name.as_deref() {
                for (i, f) in self.files.iter().enumerate() {
                    if !f.is_parent_dir() && f.name.trim_end_matches('/') == prev_name {
                        self.selected_index = i;
                        found = true;
                        break;
                    }
                }
            }
        }

        // If selected file was removed, pick the next item by previous index (or nearest available).
        if !found && !self.files.is_empty() {
            self.selected_index = prev_selected_index.min(self.files.len().saturating_sub(1));
        }

        if !self.files.is_empty() && self.selected_index >= self.files.len() {
            self.selected_index = self.files.len().saturating_sub(1);
        }

        // Never leave selection on "..": prefer next real file, else previous.
        if !self.files.is_empty() && self.files[self.selected_index].is_parent_dir() {
            let mut idx = self.selected_index;
            while idx < self.files.len() && self.files[idx].is_parent_dir() {
                idx += 1;
            }
            if idx < self.files.len() {
                self.selected_index = idx;
            } else {
                self.selected_index = self.files.len().saturating_sub(1);
                while self.selected_index > 0 && self.files[self.selected_index].is_parent_dir() {
                    self.selected_index -= 1;
                }
            }
        }

        if let Some(h) = panel_height {
            match self.view_mode {
                ViewMode::DoubleColumn => self.update_scroll_offset_double_column(h),
                ViewMode::SingleColumn => self.update_scroll_offset(h),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_tmp_under_temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "oxide_panel_climb_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn climb_to_valid_fs_path_finds_nearest_existing_dir() {
        let root = unique_tmp_under_temp();
        let _ = fs::remove_dir_all(&root);
        let deep = root.join("x").join("y").join("z");
        fs::create_dir_all(&deep).unwrap();
        assert_eq!(climb_to_valid_fs_path(&deep), deep);
        fs::remove_dir_all(root.join("x")).unwrap();
        assert_eq!(climb_to_valid_fs_path(&deep), root);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fs_path_is_listable_false_for_missing() {
        let p = unique_tmp_under_temp().join("nope");
        assert!(!fs_path_is_listable(&p));
    }
}
