use std::io;
use crate::panel::{Panel, PanelOperations, ViewMode};

/// Single source of truth for input target: panel (navigation) or command line (typing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Panel,
    CommandLine,
}

pub struct AppState {
    active_panel: usize,
    left_panel: Panel,
    right_panel: Panel,
    pub focus: Focus,
    pub command_line: String,
    pub command_line_cursor: usize,
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
        })
    }

    pub fn active_panel_mut(&mut self) -> &mut Panel {
        match self.active_panel {
            0 => &mut self.left_panel,
            1 => &mut self.right_panel,
            _ => &mut self.left_panel,
        }
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

    pub fn get_current_dir(&self) -> &str {
        match self.active_panel {
            0 => self.left_panel.get_current_dir(),
            1 => self.right_panel.get_current_dir(),
            _ => self.left_panel.get_current_dir(),
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

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Panel => Focus::CommandLine,
            Focus::CommandLine => Focus::Panel,
        };
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
