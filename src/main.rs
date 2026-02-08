use std::io;
use std::path::Path;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, size},
};
use std::time::Duration;

// Single source of truth for colors and rendering (SOLID principle)
mod styles {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use crate::FileInfo;

    pub fn file_span(name: &str) -> Span {
        Span::styled(name, Style::default().fg(Color::White))
    }

    pub fn folder_span(name: &str) -> Span {
        let folder_name = format!("{}/", name);
        Span::styled(folder_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
    }

    pub fn selected_file_span(name: &str) -> Span {
        Span::styled(name, Style::default().fg(Color::White).add_modifier(Modifier::REVERSED))
    }

    pub fn selected_folder_span(name: &str) -> Span {
        let folder_name = format!("{}/", name);
        Span::styled(folder_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::REVERSED))
    }

    // Unified line creation for both single and double column modes
    pub fn create_file_line(file: &FileInfo, is_selected: bool) -> Line {
        if is_selected {
            if file.is_dir {
                Line::from(vec![selected_folder_span(&file.name)])
            } else {
                Line::from(vec![selected_file_span(&file.name)])
            }
        } else {
            if file.is_dir {
                Line::from(vec![folder_span(&file.name)])
            } else {
                Line::from(vec![file_span(&file.name)])
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ViewMode {
    SingleColumn,
    DoubleColumn,
}

struct AppState {
    view_mode: ViewMode,
    command_input: String,
    panels_visible: bool,
    terminal_output: Vec<String>, // Screen buffer for terminal output
    max_output_lines: usize,      // Maximum lines to keep in buffer
    terminal_scroll_offset: usize, // Current scroll position for terminal
    cursor_visible: bool,         // For blinking cursor effect
    cursor_blink_counter: u8,   // Counter for blink timing
    
    // Dual panel state
    active_panel: usize,        // 0 for left, 1 for right
    left_panel: Panel,          // Left panel with independent state
    right_panel: Panel,         // Right panel with independent state
}

#[derive(Debug, Clone)]
struct FileInfo {
    name: String,
    is_dir: bool,
}

#[derive(Debug)]
struct Panel {
    current_dir: String,
    files: Vec<FileInfo>,
    selected_index: usize,
    scroll_offset: usize,
    navigation_history: Vec<(String, usize)>, // (directory_path, selected_index)
}

impl Panel {
    fn new(dir: String) -> io::Result<Self> {
        let mut panel = Self {
            current_dir: dir.clone(),
            files: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            navigation_history: Vec::new(),
        };
        panel.refresh_files()?;
        Ok(panel)
    }

    fn refresh_files(&mut self) -> io::Result<()> {
        let mut files = Vec::new();
        
        // Add parent directory entry if not at root
        if Path::new(&self.current_dir).parent().is_some() {
            files.push(FileInfo {
                name: "..".to_string(),
                is_dir: true,
            });
        }
        
        // Read current directory
        match std::fs::read_dir(&self.current_dir) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let file_name = entry.file_name()
                            .to_string_lossy()
                            .to_string();
                        
                        let metadata = entry.metadata()?;
                        let is_dir = metadata.is_dir();
                        
                        files.push(FileInfo {
                            name: file_name,
                            is_dir,
                        });
                    }
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
        
        // Sort files: directories first, then files alphabetically
        files.sort_by(|a, b| {
            if a.is_dir && !b.is_dir {
                std::cmp::Ordering::Less
            } else if a.is_dir == b.is_dir {
                a.name.cmp(&b.name)
            } else {
                std::cmp::Ordering::Greater
            }
        });
        
        self.files = files;
        self.selected_index = 0;
        self.scroll_offset = 0;
        Ok(())
    }

    fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.update_scroll_offset();
        }
    }

    fn move_down(&mut self) {
        if !self.files.is_empty() && self.selected_index < self.files.len() - 1 {
            self.selected_index += 1;
            self.update_scroll_offset();
        }
    }

    fn page_up(&mut self) {
        let panel_height = 20; // Approximate panel height
        if self.selected_index >= panel_height {
            self.selected_index -= panel_height;
            self.update_scroll_offset();
        }
    }

    fn page_down(&mut self) {
        let panel_height = 20; // Approximate panel height
        if !self.files.is_empty() && self.selected_index + panel_height < self.files.len() {
            self.selected_index += panel_height;
            self.update_scroll_offset();
        }
    }

    fn update_scroll_offset(&mut self) {
        let panel_height = 20; // Approximate panel height
        if self.selected_index >= self.scroll_offset + panel_height {
            self.scroll_offset = self.selected_index - panel_height + 1;
        } else if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }
    }

    fn smart_move_left(&mut self, panel_height: usize) {
        if self.selected_index >= self.files.len() {
            return;
        }

        let current_line = self.selected_index % panel_height;
        let current_column = self.selected_index / panel_height;
        
        // Edge case: if file list is too small
        if self.files.len() <= panel_height {
            // All files fit in first column, move to first file
            self.selected_index = 0;
        } else if current_column == 1 {
            // We're in right column, try to move to same line in left column
            let target_index = current_line;
            if target_index < self.files.len() {
                self.selected_index = target_index;
            }
        } else {
            // We're in left column, use current page_up logic
            self.page_up();
        }
    }

    fn smart_move_right(&mut self, panel_height: usize) {
        if self.selected_index >= self.files.len() {
            return;
        }

        let current_line = self.selected_index % panel_height;
        let current_column = self.selected_index / panel_height;
        
        // Edge case: if file list is too small
        if self.files.len() <= panel_height {
            // All files fit in first column, move to last file
            self.selected_index = self.files.len().saturating_sub(1);
        } else if current_column == 0 {
            // We're in left column, try to move to same line in right column
            let target_index = panel_height + current_line;
            if target_index < self.files.len() {
                self.selected_index = target_index;
            }
        } else {
            // We're in right column, use current page_down logic
            self.page_down();
        }
    }

    fn enter_directory(&mut self) -> io::Result<()> {
        if let Some(file) = self.files.get(self.selected_index) {
            if file.is_dir {
                if file.name == ".." {
                    // Going up to parent directory
                    if let Some(parent) = Path::new(&self.current_dir).parent() {
                        // Push current state to history before going up
                        self.navigation_history.push((self.current_dir.clone(), self.selected_index));
                        
                        let parent_path = parent.to_string_lossy().to_string();
                        self.current_dir = parent_path;
                        self.scroll_offset = 0;
                        self.refresh_files()?;
                        
                        // Try to find the directory we came from in the parent
                        if let Some((prev_dir, _)) = self.navigation_history.pop() {
                            // Extract just the directory name from the full path
                            if let Some(prev_name) = Path::new(&prev_dir).file_name() {
                                let prev_name_str = prev_name.to_string_lossy().to_string();
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
                } else {
                    // Entering a subdirectory - push current state to history
                    self.navigation_history.push((self.current_dir.clone(), self.selected_index));
                    
                    let new_path = Path::new(&self.current_dir).join(&file.name.trim_end_matches('/'));
                    self.current_dir = new_path.to_string_lossy().to_string();
                    self.selected_index = 0;
                    self.scroll_offset = 0;
                    self.refresh_files()?;
                }
            }
        }
        Ok(())
    }
}

impl AppState {
    fn new() -> io::Result<Self> {
        let current_dir = std::env::current_dir()?;
        let current_dir_str = current_dir.to_string_lossy().to_string();
        
        let state = Self {
            view_mode: ViewMode::DoubleColumn, // Start with double column
            command_input: String::new(),
            panels_visible: true,
            terminal_output: Vec::new(),
            max_output_lines: 1000, // Keep last 1000 lines
            terminal_scroll_offset: 0,
            cursor_visible: true,
            cursor_blink_counter: 0,
            
            // Dual panel state
            active_panel: 0,        // Start with left panel active
            left_panel: Panel::new(current_dir_str.clone())?,
            right_panel: Panel::new(current_dir_str)?,
        };
        Ok(state)
    }

    // Helper methods to work with active panel
    fn get_active_panel(&mut self) -> &mut Panel {
        match self.active_panel {
            0 => &mut self.left_panel,
            1 => &mut self.right_panel,
            _ => &mut self.left_panel,
        }
    }

    fn get_active_panel_dir(&self) -> &str {
        match self.active_panel {
            0 => &self.left_panel.current_dir,
            1 => &self.right_panel.current_dir,
            _ => &self.left_panel.current_dir,
        }
    }

    fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::SingleColumn => ViewMode::DoubleColumn,
            ViewMode::DoubleColumn => ViewMode::SingleColumn,
        };
    }

    fn toggle_panels(&mut self) {
        self.panels_visible = !self.panels_visible;
        // When hiding panels, reset to left panel active
        if !self.panels_visible {
            self.active_panel = 0;
        }
    }

    fn add_command_char(&mut self, c: char) {
        self.command_input.push(c);
    }

    fn remove_command_char(&mut self) {
        self.command_input.pop();
    }

    fn clear_command(&mut self) {
        self.command_input.clear();
    }

    fn execute_command(&mut self) -> io::Result<()> {
        if !self.command_input.is_empty() {
            // Add command to output buffer
            let command_with_prompt = format!("> {}", self.command_input);
            self.add_output_line(command_with_prompt);
            
            // Execute command and capture output
            match std::process::Command::new("sh")
                .arg("-c")
                .arg(&self.command_input)
                .current_dir(self.get_active_panel_dir())
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
            
            self.clear_command();
            
            // Add empty line to terminal output if panels are hidden
            if !self.panels_visible {
                self.add_output_line(String::new());
            }
            
            // Refresh both panels after command execution
            let _ = self.left_panel.refresh_files();
            let _ = self.right_panel.refresh_files();
        } else {
            // Empty command - just add empty line when panels are hidden
            if !self.panels_visible {
                self.add_output_line(String::new());
            }
        }
        Ok(())
    }

    fn add_output_line(&mut self, line: String) {
        self.terminal_output.push(line);
        
        // Keep only the last max_output_lines
        if self.terminal_output.len() > self.max_output_lines {
            self.terminal_output.remove(0);
        }
        
        // Auto-scroll to bottom when new output is added
        self.reset_terminal_scroll();
    }

    fn clear_output(&mut self) {
        self.terminal_output.clear();
    }

    // Terminal scrolling methods
    fn terminal_scroll_up(&mut self) {
        if self.terminal_scroll_offset > 0 {
            self.terminal_scroll_offset -= 1;
        }
    }

    fn terminal_scroll_down(&mut self) {
        let output_lines = self.terminal_output.len();
        if self.terminal_scroll_offset < output_lines.saturating_sub(1) {
            self.terminal_scroll_offset += 1;
        }
    }

    fn terminal_scroll_page_up(&mut self, page_height: usize) {
        if self.terminal_scroll_offset >= page_height {
            self.terminal_scroll_offset -= page_height;
        } else {
            self.terminal_scroll_offset = 0;
        }
    }

    fn terminal_scroll_page_down(&mut self, page_height: usize) {
        let output_lines = self.terminal_output.len();
        let max_scroll = output_lines.saturating_sub(1);
        if self.terminal_scroll_offset + page_height <= max_scroll {
            self.terminal_scroll_offset += page_height;
        } else {
            self.terminal_scroll_offset = max_scroll;
        }
    }

    fn reset_terminal_scroll(&mut self) {
        // Auto-scroll to bottom when new output is added
        self.terminal_scroll_offset = self.terminal_output.len().saturating_sub(1);
    }

    // Cursor management methods
    fn update_cursor(&mut self) {
        self.cursor_blink_counter = self.cursor_blink_counter.wrapping_add(1);
        if self.cursor_blink_counter % 8 == 0 { // Blink every 8 cycles
            self.cursor_visible = !self.cursor_visible;
        }
    }

    fn reset_cursor(&mut self) {
        self.cursor_visible = true;
        self.cursor_blink_counter = 0;
    }

    // Panel switching methods
    fn switch_panel(&mut self) {
        self.active_panel = if self.active_panel == 0 { 1 } else { 0 };
    }
}

fn draw_ui(f: &mut Frame, app: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0), // Main content area
            Constraint::Length(1), // Command line (1 line)
        ])
        .split(f.area());

    // Draw main content area
    if app.panels_visible {
        // Split main area into two panels
        let panel_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[0]);

        // Draw left panel
        draw_single_panel(f, app, panel_chunks[0], "Left Panel");
        
        // Draw right panel
        draw_single_panel(f, app, panel_chunks[1], "Right Panel");
    } else {
        // Show terminal output
        draw_terminal_output(f, app, chunks[0]);
    }

    // Always draw command line
    draw_command_line(f, app, chunks[1]);
}

fn draw_terminal_output(f: &mut Frame, app: &mut AppState, area: Rect) {
    let output_text = if app.terminal_output.is_empty() {
        "No command output yet. Type a command and press Enter to see output here."
            .to_string()
    } else {
        app.terminal_output.join("\n")
    };

    // Calculate how many lines we can show
    let available_height = area.height as usize;
    let output_lines: Vec<&str> = output_text.lines().collect();
    
    // Use scroll offset to determine which lines to show
    let start_line = if output_lines.len() > available_height {
        if app.terminal_scroll_offset >= available_height {
            app.terminal_scroll_offset - available_height + 1
        } else {
            0
        }
    } else {
        0
    };
    
    let end_line = (start_line + available_height).min(output_lines.len());
    let visible_lines = &output_lines[start_line..end_line];
    let scrollable_text = visible_lines.join("\n");

    let terminal_display = Paragraph::new(scrollable_text)
        .style(Style::default().fg(Color::Rgb(200, 200, 200))); // Light grey
    
    f.render_widget(terminal_display, area);
}

fn draw_command_line(f: &mut Frame, app: &mut AppState, area: Rect) {
    let cursor_char = if app.cursor_visible { "█" } else { " " };
    let command_with_cursor = format!("> {}{}", app.command_input, cursor_char);
    let command_line = Paragraph::new(command_with_cursor)
        .style(Style::default().fg(Color::White));
    
    f.render_widget(command_line, area);
}

fn draw_single_panel(f: &mut Frame, app: &mut AppState, area: Rect, title: &str) {
    // Get view mode before borrowing
    let view_mode = app.view_mode;
    
    // Get panel for this specific title
    let (panel, is_active_panel) = if title.contains("Left") {
        (&mut app.left_panel, app.active_panel == 0)
    } else if title.contains("Right") {
        (&mut app.right_panel, app.active_panel == 1)
    } else {
        // Fallback to active panel
        (app.get_active_panel(), true)
    };
    
    match view_mode {
        ViewMode::SingleColumn => {
            draw_single_column_view(f, panel, area, title, is_active_panel);
        }
        ViewMode::DoubleColumn => {
            draw_double_column_view(f, panel, area, title, is_active_panel);
        }
    }
}

fn draw_single_column_view(f: &mut Frame, panel: &mut Panel, area: Rect, _title: &str, _is_active_panel: bool) {
    let panel_height = area.height.saturating_sub(2) as usize; // Subtract border space
    
    // Update scroll offset based on panel height
    if panel.selected_index >= panel.scroll_offset + panel_height {
        panel.scroll_offset = panel.selected_index - panel_height + 1;
    } else if panel.selected_index < panel.scroll_offset {
        panel.scroll_offset = panel.selected_index;
    }

    // Create panel title with current file if active
    let panel_title = if _is_active_panel && !panel.files.is_empty() {
        let current_file = &panel.files[panel.selected_index];
        format!("{} - {}", panel.current_dir, current_file.name)
    } else {
        panel.current_dir.clone()
    };

    let visible_files: Vec<ListItem> = panel.files
        .iter()
        .skip(panel.scroll_offset)
        .take(panel_height)
        .enumerate()
        .map(|(i, file)| {
            let actual_index = i + panel.scroll_offset;
            let is_selected = _is_active_panel && actual_index == panel.selected_index;
            
            ListItem::new(styles::create_file_line(file, is_selected))
        })
        .collect();

    let list = List::new(visible_files)
        .block(Block::default().borders(Borders::ALL).title(panel_title));
    f.render_widget(list, area);
}

fn draw_double_column_view(f: &mut Frame, panel: &mut Panel, area: Rect, _title: &str, _is_active_panel: bool) {
    let panel_height = area.height.saturating_sub(2) as usize; // Subtract border space
    let panel_width = area.width.saturating_sub(2) as usize; // Subtract border space
    
    // Calculate column dimensions
    let files_per_column = panel_height;
    let column_width = panel_width / 2;
    
    // Update scroll offset based on panel height
    if panel.selected_index >= panel.scroll_offset + files_per_column * 2 {
        panel.scroll_offset = panel.selected_index - files_per_column * 2 + 1;
    } else if panel.selected_index < panel.scroll_offset {
        panel.scroll_offset = panel.selected_index;
    }

    // Calculate visible files based on horizontal scroll
    let total_visible_files = files_per_column * 2;
    let visible_files: Vec<_> = panel.files
        .iter()
        .skip(panel.scroll_offset)
        .take(total_visible_files)
        .collect();

    // Split into two columns
    let (left_files, right_files) = visible_files.split_at(visible_files.len().min(files_per_column));

    // Create a single block for the entire panel with current file if active
    let panel_title = if _is_active_panel && !panel.files.is_empty() {
        let current_file = &panel.files[panel.selected_index];
        format!("{} - {}", panel.current_dir, current_file.name)
    } else {
        panel.current_dir.clone()
    };
    
    let panel_block = Block::default()
        .borders(Borders::ALL)
        .title(panel_title);
    f.render_widget(panel_block, area);

    // Create inner area for content (inside borders)
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    
    // Split inner area into two columns, accounting for vertical line
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(column_width as u16),
            Constraint::Length(1), // Space for vertical line
            Constraint::Length((panel_width - column_width - 1) as u16),
        ])
        .split(inner);

    // Render left column content
    for (i, file) in left_files.iter().enumerate() {
        if i >= panel_height { break; }
        
        let actual_index = i + panel.scroll_offset;
        let is_selected = _is_active_panel && actual_index == panel.selected_index;
        
        let line = styles::create_file_line(file, is_selected);
        let paragraph = Paragraph::new(line);
        let line_area = Rect {
            x: columns[0].x,
            y: columns[0].y + i as u16,
            width: columns[0].width,
            height: 1,
        };
        f.render_widget(paragraph, line_area);
    }

    // Render right column content only if it has files
    if !right_files.is_empty() {
        for (i, file) in right_files.iter().enumerate() {
            if i >= panel_height { break; }
            
            let actual_index = i + left_files.len() + panel.scroll_offset;
            let is_selected = _is_active_panel && actual_index == panel.selected_index;
            
            let line = styles::create_file_line(file, is_selected);
            let paragraph = Paragraph::new(line);
            let line_area = Rect {
                x: columns[2].x,
                y: columns[2].y + i as u16,
                width: columns[2].width,
                height: 1,
            };
            f.render_widget(paragraph, line_area);
        }
    }

    // Draw vertical line between columns
    let vertical_line_x = columns[0].x + columns[0].width;
    for y in columns[0].y..columns[0].y + columns[0].height {
        let line_area = Rect {
            x: vertical_line_x,
            y,
            width: 1,
            height: 1,
        };
        let line = Paragraph::new("│")
            .style(Style::default().fg(Color::White));
        f.render_widget(line, line_area);
    }
}

fn main() -> Result<(), io::Error> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = AppState::new()?;

    // Main loop
    loop {
        // Update cursor for blinking effect
        app.update_cursor();
        
        // Draw UI
        terminal.draw(|f| draw_ui(f, &mut app))?;

        // Handle events
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Up => {
                        if app.panels_visible {
                            app.get_active_panel().move_up();
                        } else {
                            app.terminal_scroll_up();
                        }
                    }
                    KeyCode::Down => {
                        if app.panels_visible {
                            app.get_active_panel().move_down();
                        } else {
                            app.terminal_scroll_down();
                        }
                    }
                    KeyCode::Left => {
                        if app.panels_visible {
                            let panel_height = size().map(|(_, h)| h as usize).unwrap_or(20) - 3; // Subtract space for borders and command line
                            app.get_active_panel().smart_move_left(panel_height);
                        } else {
                            let terminal_height = size().map(|(_, h)| h as usize).unwrap_or(20);
                            app.terminal_scroll_page_up(terminal_height);
                        }
                    }
                    KeyCode::Right => {
                        if app.panels_visible {
                            let panel_height = size().map(|(_, h)| h as usize).unwrap_or(20) - 3; // Subtract space for borders and command line
                            app.get_active_panel().smart_move_right(panel_height);
                        } else {
                            let terminal_height = size().map(|(_, h)| h as usize).unwrap_or(20);
                            app.terminal_scroll_page_down(terminal_height);
                        }
                    }
                    KeyCode::Enter => {
                        if !app.command_input.is_empty() {
                            app.execute_command()?;
                        } else {
                            app.get_active_panel().enter_directory()?;
                        }
                    }
                    KeyCode::Char(c) => {
                        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                            match c {
                                'o' => {
                                    // Ctrl+O to toggle panels
                                    app.toggle_panels();
                                }
                                't' => {
                                    // Ctrl+T to toggle view mode
                                    app.toggle_view_mode();
                                }
                                'r' => {
                                    // Ctrl+R to refresh current directory
                                    let _ = app.get_active_panel().refresh_files();
                                }
                                'q' => {
                                    // Ctrl+Q to quit
                                    break;
                                }
                                _ => {
                                    // All other Ctrl+char combinations go to command line
                                    app.add_command_char(c);
                                    app.reset_cursor();
                                }
                            }
                        } else if c == '\t' {
                            // Tab key - switch between panels
                            app.switch_panel();
                        } else {
                            // Regular character input for command line
                            app.add_command_char(c);
                            app.reset_cursor(); // Reset cursor on input
                        }
                    }
                    KeyCode::Tab => {
                        // Tab key - switch between panels (alternate handling)
                        app.switch_panel();
                    }
                    KeyCode::Backspace => {
                        app.remove_command_char();
                        app.reset_cursor(); // Reset cursor on input
                    }
                    _ => {}
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
