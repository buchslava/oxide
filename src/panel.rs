use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use crate::file_ops::{FileOperations, FileInfo};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewMode {
    SingleColumn,
    DoubleColumn,
}

pub trait PanelOperations {
    fn move_up(&mut self, panel_height: usize);
    fn move_down(&mut self, panel_height: usize);
    fn page_up(&mut self, panel_height: usize);
    fn page_down(&mut self, panel_height: usize);
    fn smart_move_left(&mut self, panel_height: usize);
    fn smart_move_right(&mut self, panel_height: usize);
    fn enter_directory(&mut self) -> io::Result<()>;
    fn refresh_files(&mut self) -> io::Result<()>;
    fn update_scroll_offset(&mut self, panel_height: usize);
    fn update_scroll_offset_double_column(&mut self, panel_height: usize);
    fn get_current_dir(&self) -> &str;
    fn get_selected_file(&self) -> Option<&FileInfo>;
    fn get_selected_index(&self) -> usize;
    fn get_scroll_offset(&self) -> usize;
    fn get_files(&self) -> &[FileInfo];
    fn get_view_mode(&self) -> ViewMode;
    fn set_view_mode(&mut self, mode: ViewMode);
    /// Toggle selection (mark) of the current file and move to the next. F12 / MC Insert.
    fn toggle_mark_and_move_next(&mut self, panel_height: usize);
    /// Invert selection: all marked become unmarked, all unmarked (except "..") become marked. MC *.
    fn invert_selection(&mut self);
    fn is_marked(&self, index: usize) -> bool;
    /// Same as get_names_to_copy plus names of file before (first-1) and after (first+count) for restore after delete/move.
    fn get_names_to_copy_with_restore_neighbors(&self) -> (Vec<(String, bool)>, Option<String>, Option<String>);
}

#[derive(Debug)]
pub struct Panel {
    pub view_mode: ViewMode,
    current_dir: String,
    files: Vec<FileInfo>,
    selected_index: usize,
    scroll_offset: usize,
    navigation_history: Vec<(String, usize)>,
    /// Indices of files marked for group operations (F12 / MC Insert).
    marked_indices: HashSet<usize>,
    /// When true, show hidden files (names starting with "."). Toggled by Ctrl+H.
    show_hidden: bool,
}

impl Panel {
    pub fn new(dir: String) -> io::Result<Self> {
        let mut panel = Self {
            view_mode: ViewMode::DoubleColumn,
            current_dir: dir,
            files: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            navigation_history: Vec::new(),
            marked_indices: HashSet::new(),
            show_hidden: true,
        };
        panel.refresh_files()?;
        Ok(panel)
    }

    fn navigate_to_directory(&mut self, new_path: PathBuf) -> io::Result<()> {
        self.marked_indices.clear();
        self.navigation_history.push((self.current_dir.clone(), self.selected_index));
        self.current_dir = new_path.to_string_lossy().to_string();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.refresh_files()
    }

    fn navigate_to_parent(&mut self) -> io::Result<()> {
        if let Some(parent) = Path::new(&self.current_dir).parent() {
            self.marked_indices.clear();
            self.navigation_history.push((self.current_dir.clone(), self.selected_index));
            let parent_path = parent.to_string_lossy().to_string();
            self.current_dir = parent_path;
            self.scroll_offset = 0;
            self.refresh_files()?;

            // Try to find the directory we came from in the parent
            if let Some((prev_dir, _)) = self.navigation_history.pop() {
                if let Some(prev_name) = Path::new(&prev_dir).file_name() {
                    let prev_name_str = prev_name.to_string_lossy();
                    for (i, file) in self.files.iter().enumerate() {
                        if file.is_dir && file.name != ".." {
                            let file_name_clean = file.name.trim_end_matches('/');
                            if file_name_clean == prev_name_str {
                                self.selected_index = i;
                                break;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl PanelOperations for Panel {
    fn move_up(&mut self, panel_height: usize) {
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

    fn move_down(&mut self, panel_height: usize) {
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

    fn page_up(&mut self, panel_height: usize) {
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

    fn page_down(&mut self, panel_height: usize) {
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
                items = total.saturating_sub(files_per_page).saturating_sub(self.scroll_offset);
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
    fn smart_move_left(&mut self, panel_height: usize) {
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
    fn smart_move_right(&mut self, panel_height: usize) {
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
            if file.is_dir {
                if file.is_parent_dir() {
                    self.navigate_to_parent()?;
                } else {
                    let new_path = FileOperations::join_path(&self.current_dir, &file.name);
                    self.navigate_to_directory(new_path)?;
                }
            }
        }
        Ok(())
    }

    fn refresh_files(&mut self) -> io::Result<()> {
        self.marked_indices.clear();
        self.files = FileOperations::read_directory(&self.current_dir, self.show_hidden)?;
        self.selected_index = 0;
        self.scroll_offset = 0;
        if !self.files.is_empty() && self.selected_index >= self.files.len() {
            self.selected_index = self.files.len() - 1;
        }
        Ok(())
    }


    fn update_scroll_offset(&mut self, panel_height: usize) {
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
    fn update_scroll_offset_double_column(&mut self, panel_height: usize) {
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
        &self.current_dir
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

    fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
    }

    fn toggle_mark_and_move_next(&mut self, panel_height: usize) {
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

    fn is_marked(&self, index: usize) -> bool {
        self.marked_indices.contains(&index)
    }

    fn get_names_to_copy_with_restore_neighbors(&self) -> (Vec<(String, bool)>, Option<String>, Option<String>) {
        let files = self.get_files();
        if files.is_empty() {
            return (Vec::new(), None, None);
        }
        let (first_index, count, items) = if self.marked_indices.is_empty() {
            if let Some(f) = self.get_selected_file() {
                if !f.is_parent_dir() {
                    let idx = self.selected_index;
                    let mut v = Vec::with_capacity(1);
                    v.push((f.name.clone(), f.is_dir));
                    (idx, 1, v)
                } else {
                    return (Vec::new(), None, None);
                }
            } else {
                return (Vec::new(), None, None);
            }
        } else {
            let first_index = *self.marked_indices.iter().min().unwrap();
            let mut items = Vec::new();
            for &idx in &self.marked_indices {
                if let Some(f) = files.get(idx) {
                    if !f.is_parent_dir() {
                        items.push((f.name.clone(), f.is_dir));
                    }
                }
            }
            (first_index, items.len(), items)
        };
        let name_before = if first_index > 0 {
            let f = &files[first_index - 1];
            if !f.is_parent_dir() {
                Some(f.name.clone())
            } else {
                None
            }
        } else {
            None
        };
        let name_after = if first_index + count < files.len() {
            let f = &files[first_index + count];
            if !f.is_parent_dir() {
                Some(f.name.clone())
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
    /// Set whether hidden files (names starting with ".") are shown. Used by Ctrl+H toggle.
    pub fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
    }

    /// Set the current selection to the given index and update scroll so it is visible.
    pub fn set_selection(&mut self, index: usize, panel_height: usize) {
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
        self.marked_indices.clear();
        self.files = FileOperations::read_directory(&self.current_dir, self.show_hidden)?;
        self.selected_index = 0;
        self.scroll_offset = 0;

        let mut found = false;
        if let Some(name) = preferred_after {
            let name_trimmed = name.trim_end_matches('/');
            for (i, f) in self.files.iter().enumerate() {
                if f.name.trim_end_matches('/') == name_trimmed && !f.is_parent_dir() {
                    self.selected_index = i;
                    found = true;
                    break;
                }
            }
        }
        if !found {
            if let Some(name) = preferred_before {
                let name_trimmed = name.trim_end_matches('/');
                for (i, f) in self.files.iter().enumerate() {
                    if f.name.trim_end_matches('/') == name_trimmed && !f.is_parent_dir() {
                        self.selected_index = i;
                        found = true;
                        break;
                    }
                }
            }
        }
        // When neither preferred_after nor preferred_before was found (e.g. deleted last file, name_before was ".."),
        // select the new last file so we don't jump to the first.
        if !found && !self.files.is_empty() {
            self.selected_index = self.files.len().saturating_sub(1);
        }

        if !self.files.is_empty() && self.selected_index >= self.files.len() {
            self.selected_index = self.files.len().saturating_sub(1);
        }
        // Never leave selection on "..": choose nearest real file.
        while self.selected_index < self.files.len()
            && self.files[self.selected_index].is_parent_dir()
        {
            self.selected_index += 1;
        }
        if self.selected_index >= self.files.len() && !self.files.is_empty() {
            self.selected_index = self.files.len().saturating_sub(1);
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
