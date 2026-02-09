use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Gauge},
    Frame,
};
use crate::panel::{Panel, PanelOperations, ViewMode};
use crate::styles;
use crate::app_state::AppState;

pub struct Renderer;

impl Renderer {
    pub fn draw_ui(f: &mut Frame, app: &mut AppState) {
        if app.panels_visible() {
            // Panels visible: traditional layout with command line at bottom
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(0),
                    Constraint::Length(1),
                ])
                .split(f.area());

            let panel_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[0]);

            let active_panel = app.active_panel();
            Self::draw_single_panel(f, app.left_panel_mut(), panel_chunks[0], "Left Panel", active_panel == 0);
            Self::draw_single_panel(f, app.right_panel_mut(), panel_chunks[1], "Right Panel", active_panel == 1);
            Self::draw_command_line(f, app, chunks[1]);
        } else {
            // Panels hidden: command line is part of terminal area (independent)
            Self::draw_terminal_with_command_line(f, app, f.area());
        }
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
            // Apply scroll offset - different behavior for panels visible vs hidden
            let scroll_offset = app.get_active_terminal_scroll_offset();
            if app.panels_visible() {
                // Panels visible: show last lines by default (existing behavior)
                if scroll_offset == 0 {
                    total_lines - available_height
                } else {
                    scroll_offset.min(total_lines - available_height)
                }
            } else {
                // Panels hidden: show content from top when scrolling
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
            .filter(|s| !s.is_empty())
            .last()
            .unwrap_or("/")
            .to_string();
        
        let command_with_cursor = format!("{}: > {}{}", dir_name, app.command_input(), cursor_char);
        let command_line = Paragraph::new(command_with_cursor)
            .style(Style::default().fg(Color::White));
        
        f.render_widget(command_line, area);
    }

    fn draw_terminal_with_command_line(f: &mut Frame, app: &mut AppState, area: Rect) {
        // Split area: terminal output + scroll indicator, command line
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),    // Terminal area
                Constraint::Length(1), // Command line
            ])
            .split(area);

        // Split terminal area: content + vertical scroll indicator
        let terminal_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),    // Terminal content
                Constraint::Length(1), // Vertical scroll indicator
            ])
            .split(main_chunks[0]);

        // Draw terminal output
        Self::draw_terminal_output(f, app, terminal_chunks[0]);
        
        // Draw vertical scroll indicator
        Self::draw_vertical_scroll_indicator(f, app, terminal_chunks[1]);
        
        // Draw command line at bottom
        Self::draw_command_line(f, app, main_chunks[1]);
    }

    fn draw_vertical_scroll_indicator(f: &mut Frame, app: &AppState, area: Rect) {
        let output_lines = app.terminal_output();
        let total_lines = output_lines.len();
        let terminal_height = area.height as usize;
        
        // Only show indicator if there are multiple pages (scrolling is needed)
        if total_lines <= terminal_height || area.height < 3 {
            return; // No indicator needed for single page or too small area
        }
        
        let scroll_offset = app.get_active_terminal_scroll_offset();
        let indicator_height = area.height as usize;
        
        // Calculate scroll position (0.0 to 1.0)
        let max_scroll = total_lines.saturating_sub(1);
        let scroll_ratio = if max_scroll > 0 {
            scroll_offset as f64 / max_scroll as f64
        } else {
            0.0
        };
        
        // Calculate thumb position and size
        let thumb_size = 1; // Always show 1 character as thumb
        let thumb_position = (scroll_ratio * (indicator_height - thumb_size) as f64) as usize;
        
        // Build the vertical indicator
        let mut indicator_lines = Vec::new();
        for i in 0..indicator_height {
            if i == thumb_position {
                indicator_lines.push("█"); // Current position (thumb)
            } else {
                indicator_lines.push("░"); // Track
            }
        }
        
        let vertical_indicator = Paragraph::new(indicator_lines.join("\n"))
            .style(Style::default().fg(Color::DarkGray));
        
        f.render_widget(vertical_indicator, area);
    }

    fn draw_scroll_indicator(f: &mut Frame, app: &AppState, area: Rect) {
        let output_lines = app.terminal_output();
        let total_lines = output_lines.len();
        
        if total_lines == 0 {
            return; // No indicator needed for empty output
        }
        
        let scroll_offset = app.get_active_terminal_scroll_offset();
        let available_height = area.height as usize;
        
        // Calculate scroll position (0.0 to 1.0)
        let max_scroll = total_lines.saturating_sub(1);
        let scroll_ratio = if max_scroll > 0 {
            scroll_offset as f64 / max_scroll as f64
        } else {
            0.0
        };
        
        // Create a simple visual indicator using characters
        let indicator_width = area.width as usize;
        let indicator_position = (scroll_ratio * (indicator_width - 1) as f64) as usize;
        
        // Build the indicator string
        let mut indicator = String::with_capacity(indicator_width);
        for i in 0..indicator_width {
            if i == indicator_position {
                indicator.push('█'); // Current position
            } else if i < indicator_position {
                indicator.push('─'); // Scrolled past
            } else {
                indicator.push('░'); // Not yet scrolled
            }
        }
        
        let scroll_indicator = Paragraph::new(indicator)
            .style(Style::default().fg(Color::DarkGray));
        
        f.render_widget(scroll_indicator, area);
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
