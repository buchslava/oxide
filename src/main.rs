use std::fs;
use std::io;
use std::path::Path;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
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

// Single source of truth for colors (SOLID principle)
mod styles {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;

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
}

#[derive(Debug, Clone)]
enum ViewMode {
    SingleColumn,
    DoubleColumn,
}

struct AppState {
    current_dir: String,
    files: Vec<FileInfo>,
    selected_index: usize,
    scroll_offset: usize,
    navigation_history: Vec<(String, usize)>, // (directory_path, selected_index)
    horizontal_scroll: usize, // For horizontal scrolling between columns
    view_mode: ViewMode,
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
            horizontal_scroll: 0,
            view_mode: ViewMode::DoubleColumn, // Start with double column
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

    fn page_up(&mut self) {
        let panel_height = 20; // Approximate panel height
        if self.selected_index >= panel_height {
            self.selected_index -= panel_height;
        } else {
            self.selected_index = 0;
        }
        self.update_scroll_offset();
    }

    fn page_down(&mut self) {
        let panel_height = 20; // Approximate panel height
        if self.selected_index + panel_height < self.files.len() {
            self.selected_index += panel_height;
        } else {
            self.selected_index = self.files.len().saturating_sub(1);
        }
        self.update_scroll_offset();
    }

    fn update_scroll_offset(&mut self) {
        // This will be updated when we know the panel height
    }

    fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::SingleColumn => ViewMode::DoubleColumn,
            ViewMode::DoubleColumn => ViewMode::SingleColumn,
        };
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
    match app.view_mode {
        ViewMode::SingleColumn => {
            draw_single_column_view(f, app, area, title);
        }
        ViewMode::DoubleColumn => {
            draw_double_column_view(f, app, area, title);
        }
    }
}

fn draw_single_column_view(f: &mut Frame, app: &mut AppState, area: Rect, _title: &str) {
    let panel_height = area.height.saturating_sub(2) as usize; // Subtract border space
    
    // Update scroll offset based on panel height (same logic as double column)
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
            
            let line = if is_selected {
                if file.is_dir {
                    Line::from(vec![
                        styles::selected_folder_span(&file.name),
                    ])
                } else {
                    Line::from(vec![
                        styles::selected_file_span(&file.name),
                    ])
                }
            } else {
                Line::from(vec![
                    if file.is_dir { styles::folder_span(&file.name) } else { styles::file_span(&file.name) },
                ])
            };

            ListItem::new(line)
        })
        .collect();

    let list = List::new(visible_files)
        .block(Block::default().borders(Borders::ALL).title(app.current_dir.clone()));
    f.render_widget(list, area);

    // Draw current path at the bottom
    let path_text = Paragraph::new(app.current_dir.clone())
        .style(Style::default())
        .block(Block::default().borders(Borders::BOTTOM));
    let path_area = Rect {
        x: area.x,
        y: area.bottom() - 1,
        width: area.width,
        height: 1,
    };
    f.render_widget(path_text, path_area);
}

fn draw_double_column_view(f: &mut Frame, app: &mut AppState, area: Rect, _title: &str) {
    let panel_height = area.height.saturating_sub(2) as usize; // Subtract border space
    let panel_width = area.width.saturating_sub(2) as usize; // Subtract border space
    
    // Calculate column dimensions
    let files_per_column = panel_height;
    let column_width = panel_width / 2;
    
    // Update scroll offset based on panel height
    if app.selected_index >= app.scroll_offset + files_per_column * 2 {
        app.scroll_offset = app.selected_index - files_per_column * 2 + 1;
    } else if app.selected_index < app.scroll_offset {
        app.scroll_offset = app.selected_index;
    }

    // Calculate visible files based on horizontal scroll
    let total_visible_files = files_per_column * 2;
    let visible_files: Vec<_> = app.files
        .iter()
        .skip(app.scroll_offset)
        .take(total_visible_files)
        .collect();

    // Split into two columns
    let (left_files, right_files) = visible_files.split_at(visible_files.len().min(files_per_column));

    // Create a single block for the entire panel with current path as title
    let panel = Block::default()
        .borders(Borders::ALL)
        .title(app.current_dir.clone());
    f.render_widget(panel, area);

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
        
        let actual_index = i + app.scroll_offset;
        let is_selected = actual_index == app.selected_index;
        
        let line = if is_selected {
            if file.is_dir {
                styles::selected_folder_span(&file.name)
            } else {
                styles::selected_file_span(&file.name)
            }
        } else {
            if file.is_dir {
                styles::folder_span(&file.name)
            } else {
                styles::file_span(&file.name)
            }
        };

        let paragraph = Paragraph::new(line);
        let line_area = Rect {
            x: columns[0].x,
            y: columns[0].y + i as u16,
            width: columns[0].width,
            height: 1,
        };
        f.render_widget(paragraph, line_area);
    }

    // Render right column content
    for (i, file) in right_files.iter().enumerate() {
        if i >= panel_height { break; }
        
        let actual_index = i + left_files.len() + app.scroll_offset;
        let is_selected = actual_index == app.selected_index;
        
        let line = if is_selected {
            if file.is_dir {
                styles::selected_folder_span(&file.name)
            } else {
                styles::selected_file_span(&file.name)
            }
        } else {
            if file.is_dir {
                styles::folder_span(&file.name)
            } else {
                styles::file_span(&file.name)
            }
        };

        let paragraph = Paragraph::new(line);
        let line_area = Rect {
            x: columns[2].x,
            y: columns[2].y + i as u16,
            width: columns[2].width,
            height: 1,
        };
        f.render_widget(paragraph, line_area);
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
        // Draw UI
        terminal.draw(|f| draw_ui(f, &mut app))?;

        // Handle events
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => app.move_up(),
                    KeyCode::Down | KeyCode::Char('j') => app.move_down(),
                    KeyCode::Left | KeyCode::Char('h') => {
                        app.page_up();
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        app.page_down();
                    }
                    KeyCode::Enter => {
                        app.enter_directory()?;
                    }
                    KeyCode::Char('t') => {
                        // Toggle view mode between single and double column
                        app.toggle_view_mode();
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
