use std::fs;
use std::io;
use std::path::Path;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::time::Duration;

struct AppState {
    current_dir: String,
    files: Vec<FileInfo>,
    selected_index: usize,
    scroll_offset: usize,
    navigation_history: Vec<(String, usize)>, // (directory_path, selected_index)
}

#[derive(Debug, Clone)]
struct FileInfo {
    name: String,
    is_dir: bool,
    size: u64,
}

impl AppState {
    fn new() -> io::Result<Self> {
        let current_dir = std::env::current_dir()?;
        let mut state = Self {
            current_dir: current_dir.to_string_lossy().to_string(),
            files: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            navigation_history: Vec::new(),
        };
        state.refresh_files()?;
        Ok(state)
    }

    fn refresh_files(&mut self) -> io::Result<()> {
        let mut files = Vec::new();
        
        // Add parent directory entry if not at root
        if Path::new(&self.current_dir).parent().is_some() {
            files.push(FileInfo {
                name: "..".to_string(),
                is_dir: true,
                size: 0,
            });
        }

        let entries = fs::read_dir(&self.current_dir)?;
        let mut entries_vec: Vec<_> = entries.collect::<Result<Vec<_>, _>>()?;
        
        // Sort entries: directories first, then files, both alphabetically
        entries_vec.sort_by(|a, b| {
            let a_is_dir = a.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            let b_is_dir = b.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.file_name().cmp(&b.file_name()),
            }
        });

        for entry in entries_vec {
            let metadata = entry.metadata()?;
            let name = entry.file_name().to_string_lossy().to_string();
            
            files.push(FileInfo {
                name,
                is_dir: metadata.is_dir(),
                size: metadata.len(),
            });
        }

        self.files = files;
        
        // Adjust selected index if needed
        if self.selected_index >= self.files.len() {
            self.selected_index = self.files.len().saturating_sub(1);
        }
        
        Ok(())
    }

    fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.update_scroll_offset();
        }
    }

    fn move_down(&mut self) {
        if self.selected_index < self.files.len().saturating_sub(1) {
            self.selected_index += 1;
            self.update_scroll_offset();
        }
    }

    fn update_scroll_offset(&mut self) {
        // This will be updated when we know the panel height
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

fn draw_ui(f: &mut Frame, app: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(f.area());

    draw_file_panel(f, app, chunks[0], "Left Panel");
    
    // Right panel (empty for now)
    let right_panel = Block::default()
        .borders(Borders::ALL)
        .title("Right Panel");
    f.render_widget(right_panel, chunks[1]);
}

fn draw_file_panel(f: &mut Frame, app: &mut AppState, area: Rect, title: &str) {
    let panel_height = area.height.saturating_sub(2) as usize; // Subtract border space
    
    // Update scroll offset based on panel height
    if app.selected_index >= app.scroll_offset + panel_height {
        app.scroll_offset = app.selected_index - panel_height + 1;
    } else if app.selected_index < app.scroll_offset {
        app.scroll_offset = app.selected_index;
    }

    let visible_files: Vec<ListItem> = app.files
        .iter()
        .skip(app.scroll_offset)
        .take(panel_height)
        .enumerate()
        .map(|(i, file)| {
            let actual_index = i + app.scroll_offset;
            let is_selected = actual_index == app.selected_index;
            
            let (name, style) = if file.is_dir {
                (format!("{}/", file.name), Style::default().fg(Color::Blue))
            } else {
                (file.name.clone(), Style::default().fg(Color::White))
            };

            let line = if is_selected {
                Line::from(vec![
                    Span::styled(">", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                    Span::styled(name, style.add_modifier(Modifier::REVERSED)),
                ])
            } else {
                Line::from(vec![
                    Span::raw(" "),
                    Span::raw(" "),
                    Span::styled(name, style),
                ])
            };

            ListItem::new(line)
        })
        .collect();

    let list = List::new(visible_files)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default());

    f.render_widget(list, area);

    // Draw current path at the bottom
    let path_text = Paragraph::new(app.current_dir.clone())
        .style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::BOTTOM));
    let path_area = Rect {
        x: area.x,
        y: area.bottom() - 1,
        width: area.width,
        height: 1,
    };
    f.render_widget(path_text, path_area);
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
        // Draw UI
        terminal.draw(|f| draw_ui(f, &mut app))?;

        // Handle events
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                    KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                    KeyCode::Enter | KeyCode::Char('l') => {
                        app.enter_directory()?;
                    }
                    KeyCode::Char('h') => {
                        // Go to parent directory using navigation history
                        if let Some(parent) = Path::new(&app.current_dir).parent() {
                            // Push current state to history
                            app.navigation_history.push((app.current_dir.clone(), app.selected_index));
                            
                            let parent_path = parent.to_string_lossy().to_string();
                            app.current_dir = parent_path;
                            app.scroll_offset = 0;
                            app.refresh_files()?;
                            
                            // Try to find the directory we came from
                            if let Some((prev_dir, _)) = app.navigation_history.pop() {
                                if let Some(prev_name) = Path::new(&prev_dir).file_name() {
                                    let prev_name_str = prev_name.to_string_lossy().to_string();
                                    for (i, file) in app.files.iter().enumerate() {
                                        if file.is_dir && file.name != ".." {
                                            let file_name_clean = file.name.trim_end_matches('/');
                                            if file_name_clean == prev_name_str {
                                                app.selected_index = i;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Char('r') => {
                        // Refresh current directory
                        app.refresh_files()?;
                    }
                    KeyCode::Char('q') => break,
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
