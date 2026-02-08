use std::io;
use std::path::Path;
use crate::panel::{Panel, PanelOperations, ViewMode};
use crate::file_ops::FileOperations;

pub struct AppState {
    command_input: String,
    panels_visible: bool,
    terminal_output: Vec<String>,
    max_output_lines: usize,
    terminal_scroll_offset: usize,
    cursor_visible: bool,
    cursor_blink_counter: u8,
    active_panel: usize,
    left_panel: Panel,
    right_panel: Panel,
}

impl AppState {
    pub fn new() -> io::Result<Self> {
        let current_dir = std::env::current_dir()?;
        let current_dir_str = current_dir.to_string_lossy().to_string();
        
        let state = Self {
            command_input: String::new(),
            panels_visible: true,
            terminal_output: Vec::new(),
            max_output_lines: 1000,
            terminal_scroll_offset: 0,
            cursor_visible: true,
            cursor_blink_counter: 0,
            active_panel: 0,
            left_panel: Panel::new(current_dir_str.clone())?,
            right_panel: Panel::new(current_dir_str)?,
        };
        Ok(state)
    }

    // Panel access methods
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

    pub fn panels_visible(&self) -> bool {
        self.panels_visible
    }

    // Command line methods
    pub fn command_input(&self) -> &str {
        &self.command_input
    }

    pub fn add_command_char(&mut self, c: char) {
        self.command_input.push(c);
    }

    pub fn remove_command_char(&mut self) {
        self.command_input.pop();
    }

    pub fn clear_command(&mut self) {
        self.command_input.clear();
    }

    // Terminal output methods
    pub fn terminal_output(&self) -> &[String] {
        &self.terminal_output
    }

    pub fn terminal_scroll_offset(&self) -> usize {
        self.terminal_scroll_offset
    }

    pub fn add_output_line(&mut self, line: String) {
        self.terminal_output.push(line);
        
        if self.terminal_output.len() > self.max_output_lines {
            self.terminal_output.remove(0);
        }
        
        self.reset_terminal_scroll();
    }

    // Terminal scrolling methods
    pub fn terminal_scroll_up(&mut self) {
        if self.terminal_scroll_offset > 0 {
            self.terminal_scroll_offset -= 1;
        }
    }

    pub fn terminal_scroll_down(&mut self) {
        let output_lines = self.terminal_output.len();
        if self.terminal_scroll_offset < output_lines.saturating_sub(1) {
            self.terminal_scroll_offset += 1;
        }
    }

    pub fn terminal_scroll_page_up(&mut self, page_height: usize) {
        if self.terminal_scroll_offset < page_height {
            self.terminal_scroll_offset = 0;
            return;
        }
        self.terminal_scroll_offset -= page_height;
    }

    pub fn terminal_scroll_page_down(&mut self, page_height: usize) {
        let output_lines = self.terminal_output.len();
        let max_scroll = output_lines.saturating_sub(1);
        if self.terminal_scroll_offset + page_height > max_scroll {
            self.terminal_scroll_offset = max_scroll;
            return;
        }
        self.terminal_scroll_offset += page_height;
    }

    fn reset_terminal_scroll(&mut self) {
        self.terminal_scroll_offset = self.terminal_output.len().saturating_sub(1);
    }

    // Cursor management methods
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn update_cursor(&mut self) {
        self.cursor_blink_counter = self.cursor_blink_counter.wrapping_add(1);
        if self.cursor_blink_counter % 8 == 0 {
            self.cursor_visible = !self.cursor_visible;
        }
    }

    pub fn reset_cursor(&mut self) {
        self.cursor_visible = true;
        self.cursor_blink_counter = 0;
    }

    // Panel management methods
    pub fn toggle_view_mode(&mut self) {
        let panel = self.active_panel_mut();
        let new_mode = match panel.get_view_mode() {
            ViewMode::SingleColumn => ViewMode::DoubleColumn,
            ViewMode::DoubleColumn => ViewMode::SingleColumn,
        };
        panel.set_view_mode(new_mode);
    }

    pub fn toggle_panels(&mut self) {
        self.panels_visible = !self.panels_visible;
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

    pub fn execute_command(&mut self) -> io::Result<()> {
        if self.command_input.is_empty() {
            if !self.panels_visible {
                self.add_output_line(String::new());
            }
            return Ok(());
        }

        let command_with_prompt = format!("> {}", self.command_input);
        self.add_output_line(command_with_prompt);
        
        if self.command_input.trim_start().starts_with("cd ") {
            self.handle_cd_command()?;
        } else {
            self.handle_shell_command()?;
        }
        
        self.clear_command();
        
        if !self.panels_visible {
            self.add_output_line(String::new());
        }
        
        // Refresh both panels after command execution
        let _ = self.left_panel.refresh_files();
        let _ = self.right_panel.refresh_files();
        Ok(())
    }

    fn handle_cd_command(&mut self) -> io::Result<()> {
        let cd_target = self.command_input.trim_start()[3..].trim();
        let current_dir = {
            // Get the current directory from the active panel
            let panel = match self.active_panel {
                0 => &self.left_panel,
                1 => &self.right_panel,
                _ => &self.left_panel,
            };
            panel.get_current_dir().to_string()
        };
        
        let new_dir = if cd_target == ".." {
            if let Some(parent) = Path::new(&current_dir).parent() {
                parent.to_string_lossy().to_string()
            } else {
                current_dir
            }
        } else if cd_target.starts_with('/') {
            cd_target.to_string()
        } else {
            let new_path = Path::new(&current_dir).join(cd_target);
            new_path.to_string_lossy().to_string()
        };
        
        if FileOperations::path_exists(&new_dir) && FileOperations::is_directory(&new_dir) {
            {
                let active_panel = self.active_panel_mut();
                active_panel.set_current_dir(new_dir.clone());
                active_panel.refresh_files()?;
            }
            self.add_output_line(format!("Changed to: {}", new_dir));
        } else {
            self.add_output_line(format!("Error: Directory '{}' does not exist", new_dir));
        }
        Ok(())
    }

    fn handle_shell_command(&mut self) -> io::Result<()> {
        let current_dir = {
            // Get the current directory from the active panel
            let panel = match self.active_panel {
                0 => &self.left_panel,
                1 => &self.right_panel,
                _ => &self.left_panel,
            };
            panel.get_current_dir().to_string()
        };
        
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(&self.command_input)
            .current_dir(&current_dir)
            .output()
        {
            Ok(output) => {
                if !output.stdout.is_empty() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    for line in stdout.lines() {
                        self.add_output_line(line.to_string());
                    }
                }
                if !output.stderr.is_empty() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    for line in stderr.lines() {
                        self.add_output_line(format!("Error: {}", line));
                    }
                }
            }
            Err(e) => {
                self.add_output_line(format!("Failed to execute command: {}", e));
            }
        }
        Ok(())
    }
}
