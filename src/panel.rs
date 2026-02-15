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
    fn set_current_dir(&mut self, dir: String);
    fn get_debug_info(&self) -> String;
}

#[derive(Debug)]
pub struct Panel {
    pub view_mode: ViewMode,
    current_dir: String,
    files: Vec<FileInfo>,
    selected_index: usize,
    scroll_offset: usize,
    navigation_history: Vec<(String, usize)>,
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
        };
        panel.refresh_files()?;
        Ok(panel)
    }

    fn navigate_to_directory(&mut self, new_path: PathBuf) -> io::Result<()> {
        self.navigation_history.push((self.current_dir.clone(), self.selected_index));
        self.current_dir = new_path.to_string_lossy().to_string();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.refresh_files()
    }

    fn navigate_to_parent(&mut self) -> io::Result<()> {
        if let Some(parent) = Path::new(&self.current_dir).parent() {
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
        self.files = FileOperations::read_directory(&self.current_dir)?;
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

    fn get_debug_info(&self) -> String {
        format!("idx:{}|scroll:{}|files:{}|page:{}", 
            self.get_selected_index(),
            self.get_scroll_offset(),
            self.get_files().len(),
            if self.get_view_mode() == ViewMode::DoubleColumn {
                let h = 20; // panel height
                let page = (self.get_selected_index() / (h * 2)) * (h * 2);
                format!("{}|page{}", page, page)
            } else {
                "single".to_string()
            })
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

    fn set_current_dir(&mut self, dir: String) {
        self.current_dir = dir;
    }
}

impl Panel {
    /// Re-read directory and try to restore selection by file name (e.g. after running a command).
    pub fn refresh_files_restore_selection(&mut self, preferred_name: Option<&str>) -> io::Result<()> {
        self.files = FileOperations::read_directory(&self.current_dir)?;
        self.selected_index = 0;
        self.scroll_offset = 0;
        if let Some(name) = preferred_name {
            let name_trimmed = name.trim_end_matches('/');
            for (i, f) in self.files.iter().enumerate() {
                let fname = f.name.trim_end_matches('/');
                if fname == name_trimmed {
                    self.selected_index = i;
                    break;
                }
            }
        }
        if !self.files.is_empty() && self.selected_index >= self.files.len() {
            self.selected_index = self.files.len().saturating_sub(1);
        }
        Ok(())
    }
}
