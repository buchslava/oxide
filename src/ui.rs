use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use crate::panel::{Panel, PanelOperations, ViewMode};
use crate::styles;
use crate::app_state::AppState;

pub struct Renderer;

impl Renderer {
    pub fn draw_ui(f: &mut Frame, app: &mut AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(f.area());

        if app.panels_visible() {
            let panel_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[0]);

            let active_panel = app.active_panel();
            Self::draw_single_panel(f, app.left_panel_mut(), panel_chunks[0], "Left Panel", active_panel == 0);
            Self::draw_single_panel(f, app.right_panel_mut(), panel_chunks[1], "Right Panel", active_panel == 1);
        } else {
            Self::draw_terminal_output(f, app, chunks[0]);
        }

        Self::draw_command_line(f, app, chunks[1]);
    }

    fn draw_terminal_output(f: &mut Frame, app: &AppState, area: Rect) {
        let output_lines = app.terminal_output();
        
        if output_lines.is_empty() {
            return; // Don't draw anything when there's no output
        }

        let available_height = area.height as usize;
        let total_lines = output_lines.len();
        
        // Calculate which lines to show, ensuring content appears from bottom
        let lines_to_show = available_height.min(total_lines);
        let start_index = if total_lines > available_height {
            // Apply scroll offset, but ensure we show the last lines when not scrolling
            let scroll_offset = app.terminal_scroll_offset();
            if scroll_offset == 0 {
                total_lines - available_height // Show last lines by default
            } else {
                scroll_offset.min(total_lines - available_height)
            }
        } else {
            0
        };
        
        let end_index = start_index + lines_to_show;
        let visible_lines: Vec<&str> = output_lines[start_index..end_index]
            .iter()
            .map(|s| s.as_str())
            .collect();
        
        let scrollable_text = visible_lines.join("\n");

        // Add padding to push content to bottom
        let available_height = area.height as usize;
        let padding_lines = available_height.saturating_sub(visible_lines.len());
        let padded_text = if padding_lines > 0 {
            let mut padding = String::new();
            for _ in 0..padding_lines {
                padding.push('\n');
            }
            padding + &scrollable_text
        } else {
            scrollable_text
        };

        let terminal_display = Paragraph::new(padded_text)
            .style(Style::default().fg(Color::Rgb(200, 200, 200)));
        
        f.render_widget(terminal_display, area);
    }

    fn draw_command_line(f: &mut Frame, app: &AppState, area: Rect) {
        let cursor_char = if app.cursor_visible() { "█" } else { " " };
        
        // Get current directory name (last component of the path)
        let current_dir = app.get_current_dir();
        let dir_name = current_dir
            .split('/')
            .last()
            .unwrap_or(current_dir)
            .to_string();
        
        let command_with_cursor = format!("{}: > {}{}", dir_name, app.command_input(), cursor_char);
        let command_line = Paragraph::new(command_with_cursor)
            .style(Style::default().fg(Color::White));
        
        f.render_widget(command_line, area);
    }

    fn draw_single_panel(f: &mut Frame, panel: &mut Panel, area: Rect, _title: &str, is_active_panel: bool) {
        match panel.get_view_mode() {
            ViewMode::SingleColumn => {
                Self::draw_single_column_view(f, panel, area, is_active_panel);
            }
            ViewMode::DoubleColumn => {
                Self::draw_double_column_view(f, panel, area, is_active_panel);
            }
        }
    }

    fn draw_single_column_view(f: &mut Frame, panel: &mut Panel, area: Rect, is_active_panel: bool) {
        let panel_height = area.height.saturating_sub(2) as usize;
        
        // Update scroll offset based on panel height
        if panel.get_selected_index() >= panel.get_scroll_offset() + panel_height {
            // This is a bit of a hack - we need to modify the scroll offset
            // but we don't have mutable access to the internal field
            // For now, we'll rely on the panel's own scroll management
        }

        let panel_title = if is_active_panel && !panel.get_files().is_empty() {
            if let Some(current_file) = panel.get_selected_file() {
                format!("{} - {}", panel.get_current_dir(), current_file.name)
            } else {
                panel.get_current_dir().to_string()
            }
        } else {
            panel.get_current_dir().to_string()
        };

        let visible_files: Vec<ListItem> = panel.get_files()
            .iter()
            .skip(panel.get_scroll_offset())
            .take(panel_height)
            .enumerate()
            .map(|(i, file)| {
                let actual_index = i + panel.get_scroll_offset();
                let is_selected = is_active_panel && actual_index == panel.get_selected_index();
                
                ListItem::new(styles::create_file_line(file, is_selected))
            })
            .collect();

        let list = List::new(visible_files)
            .block(Block::default().borders(Borders::ALL).title(panel_title));
        f.render_widget(list, area);
    }

    fn draw_double_column_view(f: &mut Frame, panel: &mut Panel, area: Rect, is_active_panel: bool) {
        let panel_height = area.height.saturating_sub(2) as usize;
        let panel_width = area.width.saturating_sub(2) as usize;
        
        let files_per_column = panel_height;
        let column_width = panel_width / 2;
        let files_per_page = files_per_column * 2;
        
        // Update scroll offset for double column view
        panel.update_scroll_offset_double_column(panel_height);
        
        let panel_title = if is_active_panel && !panel.get_files().is_empty() {
            if let Some(current_file) = panel.get_selected_file() {
                format!("{} - {}", panel.get_current_dir(), current_file.name)
            } else {
                panel.get_current_dir().to_string()
            }
        } else {
            panel.get_current_dir().to_string()
        };
        
        let panel_block = Block::default()
            .borders(Borders::ALL)
            .title(panel_title);
        f.render_widget(panel_block, area);

        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(column_width as u16),
                Constraint::Length(1),
                Constraint::Length((panel_width - column_width - 1) as u16),
            ])
            .split(inner);

        let visible_files: Vec<_> = panel.get_files()
            .iter()
            .skip(panel.get_scroll_offset())
            .take(files_per_page)
            .collect();

        let (left_files, right_files) = visible_files.split_at(visible_files.len().min(files_per_column));

        // Render left column
        for (i, file) in left_files.iter().enumerate() {
            if i >= panel_height { break; }
            
            let actual_index = i + panel.get_scroll_offset();
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            
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

        // Render right column
        for (i, file) in right_files.iter().enumerate() {
            if i >= panel_height { break; }
            
            let actual_index = i + left_files.len() + panel.get_scroll_offset();
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            
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
}
