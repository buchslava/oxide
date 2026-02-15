use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use crate::file_ops::FileInfo;
use crate::panel::{Panel, PanelOperations, ViewMode};
use crate::styles;
use crate::app_state::{AppState, Focus};

pub struct Renderer;

/// Truncate file display to fit column width (chars). Prevents wrapping/uglification in double-column view.
/// Prefix: "/" for folders, "*" for executables, " " for regular files (as in screenshot).
fn truncate_for_width(file: &FileInfo, max_width: usize) -> String {
    let full = if file.is_dir {
        format!("/{}", file.name)
    } else if file.is_executable {
        format!("*{}", file.name)
    } else {
        format!(" {}", file.name)
    };
    let w = max_width.saturating_sub(1); // leave room for "…"
    if full.chars().count() <= max_width {
        full
    } else {
        format!("{}…", full.chars().take(w).collect::<String>())
    }
}

impl Renderer {
    /// MC-style: panels + status + command line. Command runs in original terminal.
    pub fn draw_ui(f: &mut Frame, app: &mut AppState) {
        Self::draw_panels_view(f, app);
    }

    fn draw_panels_view(f: &mut Frame, app: &mut AppState) {
        let area = f.area();
        let panel_height = area.height.saturating_sub(2); // 1 status + 1 command line
        let left_w = area.width / 2;
        let right_w = area.width.saturating_sub(left_w);

        let left_panel = Rect {
            x: area.x,
            y: area.y,
            width: left_w,
            height: panel_height,
        };
        let right_panel = Rect {
            x: area.x + left_w,
            y: area.y,
            width: right_w,
            height: panel_height,
        };
        let status_rect = Rect {
            x: area.x,
            y: area.y + panel_height,
            width: area.width,
            height: 1,
        };
        let command_rect = Rect {
            x: area.x,
            y: area.y + panel_height + 1,
            width: area.width,
            height: 1,
        };

        let active_panel = app.active_panel();
        Self::draw_single_panel(f, app.left_panel_mut(), left_panel, "Left Panel", active_panel == 0);
        Self::draw_single_panel(f, app.right_panel_mut(), right_panel, "Right Panel", active_panel == 1);

        Self::draw_status_line(f, app, status_rect);
        Self::draw_command_line(f, app, command_rect);
    }

    fn draw_command_line(f: &mut Frame, app: &AppState, area: Rect) {
        let prompt = "$ ";
        let line = format!("{}{}", prompt, app.command_line);
        let is_focused = app.focus == Focus::CommandLine;
        let style = if is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let p = Paragraph::new(line.clone()).style(style);
        f.render_widget(p, area);
        if is_focused {
            let cursor_x = (prompt.len() + app.command_line_cursor.min(app.command_line.len())) as u16;
            if cursor_x < area.width {
                f.set_cursor_position((area.x + cursor_x, area.y));
            }
        }
    }

    fn draw_status_line(f: &mut Frame, app: &AppState, area: Rect) {
        let dir_name = app
            .get_current_dir()
            .split('/')
            .filter(|s| !s.is_empty())
            .last()
            .unwrap_or("/");
        let line = format!(
            " {}  |  Tab: switch panel / leave cmd  |  F6 / Type: command line  |  Ctrl+O: shell  |  F10: quit ",
            dir_name
        );
        let p = Paragraph::new(line).style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, area);
    }

    fn draw_single_panel(f: &mut Frame, panel: &mut Panel, area: Rect, _title: &str, is_active_panel: bool) {
        match panel.get_view_mode() {
            ViewMode::SingleColumn => Self::draw_single_column_view(f, panel, area, is_active_panel),
            ViewMode::DoubleColumn => Self::draw_double_column_view(f, panel, area, is_active_panel),
        }
    }

    fn draw_single_column_view(f: &mut Frame, panel: &mut Panel, area: Rect, is_active_panel: bool) {
        let panel_height = (area.height.saturating_sub(2) as usize).max(1);
        let panel_title = if is_active_panel && !panel.get_files().is_empty() {
            if let Some(current_file) = panel.get_selected_file() {
                format!("{} - {}", panel.get_current_dir(), current_file.name)
            } else {
                panel.get_current_dir().to_string()
            }
        } else {
            panel.get_current_dir().to_string()
        };

        let files = panel.get_files();
        let scroll = panel.get_scroll_offset().min(files.len().saturating_sub(1).max(0));
        let visible_files: Vec<ListItem> = files
            .iter()
            .skip(scroll)
            .take(panel_height)
            .enumerate()
            .map(|(i, file)| {
                let actual_index = i + scroll;
                let is_selected = is_active_panel && actual_index == panel.get_selected_index();
                ListItem::new(styles::create_file_line(file, is_selected))
            })
            .collect();

        let list = List::new(visible_files)
            .block(Block::default().borders(Borders::ALL).title(panel_title));
        f.render_widget(list, area);
    }

    fn draw_double_column_view(f: &mut Frame, panel: &mut Panel, area: Rect, is_active_panel: bool) {
        let panel_height = (area.height.saturating_sub(2) as usize).max(1);
        let files_per_column = panel_height;
        let files_per_page = files_per_column * 2;

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
        let col_w = (inner.width / 2).max(1);
        let left_col = Rect {
            x: inner.x,
            y: inner.y,
            width: col_w,
            height: inner.height,
        };
        let right_col = Rect {
            x: inner.x + col_w + 1,
            y: inner.y,
            width: inner.width.saturating_sub(col_w + 1).max(1),
            height: inner.height,
        };
        let vertical_line_x = inner.x + col_w;

        let files = panel.get_files();
        let max_scroll = files.len().saturating_sub(files_per_page).max(0);
        let scroll = panel.get_scroll_offset().min(max_scroll);
        let visible_files: Vec<_> = files
            .iter()
            .skip(scroll)
            .take(files_per_page)
            .collect();
        let (left_files, right_files) = visible_files.split_at(visible_files.len().min(files_per_column));

        let max_left_w = (left_col.width as usize).max(1);
        let max_right_w = (right_col.width as usize).max(1);

        for (i, file) in left_files.iter().enumerate() {
            if i >= panel_height {
                break;
            }
            let actual_index = i + scroll;
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            let display = truncate_for_width(file, max_left_w);
            let line = styles::create_file_line_from_display(&display, file.is_dir, file.is_executable, is_selected);
            let line_area = Rect {
                x: left_col.x,
                y: left_col.y + i as u16,
                width: left_col.width,
                height: 1,
            };
            f.render_widget(Paragraph::new(line), line_area);
        }
        for (i, file) in right_files.iter().enumerate() {
            if i >= panel_height {
                break;
            }
            let actual_index = i + left_files.len() + scroll;
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            let display = truncate_for_width(file, max_right_w);
            let line = styles::create_file_line_from_display(&display, file.is_dir, file.is_executable, is_selected);
            let line_area = Rect {
                x: right_col.x,
                y: right_col.y + i as u16,
                width: right_col.width,
                height: 1,
            };
            f.render_widget(Paragraph::new(line), line_area);
        }

        for y in left_col.y..left_col.y + left_col.height {
            f.render_widget(
                Paragraph::new("│").style(Style::default().fg(Color::White)),
                Rect {
                    x: vertical_line_x,
                    y,
                    width: 1,
                    height: 1,
                },
            );
        }
    }
}
