//! File viewer (F3): view file as text or hex dump. ESC to close.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app_state::{AppState, ViewerMode, ViewerScreenState};
use crate::events::AppAction;
use crate::file_ops::FileOperations;
use crate::panel::PanelOperations;

const HEX_BYTES_PER_LINE: usize = 16;

/// Close the viewer and return to panels.
pub fn close_viewer(app: &mut AppState) {
    app.viewer_screen = None;
}

/// Open the currently selected file in the viewer. Returns true if opened.
pub fn open_viewer(app: &mut AppState) -> bool {
    let cwd = app.get_current_dir().to_string();
    if let Some(file) = app.active_panel_mut().get_selected_file() {
        if !file.is_dir && !file.is_parent_dir() {
            let path = FileOperations::join_path(&cwd, &file.name);
            let file_path_str = path.to_string_lossy().to_string();
            let content = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => return false,
            };
            app.viewer_screen = Some(ViewerScreenState {
                file_path: file_path_str,
                content,
                view_mode: ViewerMode::Text,
                scroll: 0,
                area: Rect::default(),
            });
            return true;
        }
    }
    false
}

/// Visible lines in viewer (area minus block borders and status line).
fn visible_lines(v: &ViewerScreenState) -> usize {
    v.area.height.saturating_sub(3).max(1) as usize
}

/// Handle key when viewer is open. Returns Some(action) when handled, None if not in viewer.
pub fn handle_viewer_key(app: &mut AppState, key: crossterm::event::KeyEvent) -> Option<AppAction> {
    use crossterm::event::KeyCode;
    let v = app.viewer_screen.as_mut()?;
    let height = visible_lines(v);

    if key.code == KeyCode::Esc {
        return Some(AppAction::ViewerClose);
    }
    if key.code == KeyCode::Char('h') || key.code == KeyCode::Char('H') {
        v.view_mode = match v.view_mode {
            ViewerMode::Text => ViewerMode::Hex,
            ViewerMode::Hex => ViewerMode::Text,
        };
        v.scroll = 0;
        return Some(AppAction::Continue);
    }

    let total_lines = line_count(v);
    let max_scroll = total_lines.saturating_sub(height).max(0);

    match key.code {
        KeyCode::Up => {
            v.scroll = v.scroll.saturating_sub(1);
        }
        KeyCode::Down => {
            v.scroll = (v.scroll + 1).min(max_scroll);
        }
        KeyCode::PageUp => {
            v.scroll = v.scroll.saturating_sub(height);
        }
        KeyCode::PageDown => {
            v.scroll = (v.scroll + height).min(max_scroll);
        }
        KeyCode::Home => v.scroll = 0,
        KeyCode::End => v.scroll = max_scroll,
        _ => {}
    }
    Some(AppAction::Continue)
}

fn line_count(v: &ViewerScreenState) -> usize {
    match v.view_mode {
        ViewerMode::Text => text_lines(v).len(),
        ViewerMode::Hex => hex_lines(v).len(),
    }
}

fn text_lines(v: &ViewerScreenState) -> Vec<String> {
    let s = String::from_utf8_lossy(&v.content);
    let mut lines: Vec<String> = s.lines().map(|l| l.to_string()).collect();
    if v.content.ends_with(b"\n") || (!v.content.is_empty() && !s.ends_with('\n')) {
        if !v.content.is_empty() && !s.ends_with('\n') && !lines.is_empty() {
            // last line has no newline
        } else if v.content.ends_with(b"\n") && s.ends_with('\n') {
            lines.push(String::new());
        }
    }
    if lines.is_empty() && !v.content.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn hex_lines(v: &ViewerScreenState) -> Vec<String> {
    let mut lines = Vec::new();
    let bytes = &v.content;
    for chunk in bytes.chunks(HEX_BYTES_PER_LINE) {
        let offset = lines.len() * HEX_BYTES_PER_LINE;
        let addr = format!("{:08x}: ", offset);
        let hex_part: Vec<String> = chunk
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        let hex_str = if hex_part.len() <= 8 {
            hex_part.join(" ")
        } else {
            format!(
                "{}  {}",
                hex_part[..8].join(" "),
                hex_part[8..].join(" ")
            )
        };
        let ascii: String = chunk
            .iter()
            .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
            .collect();
        let padding = HEX_BYTES_PER_LINE - chunk.len();
        let pad_hex = "   ".repeat(padding);
        let pad_ascii = " ".repeat(padding);
        lines.push(format!(
            "{}{}{}  |{}{}|",
            addr, hex_str, pad_hex, ascii, pad_ascii
        ));
    }
    if lines.is_empty() && !bytes.is_empty() {
        lines.push("00000000: (empty)".to_string());
    }
    if lines.is_empty() {
        lines.push("(empty file)".to_string());
    }
    lines
}

/// Draw the viewer (text or hex) with scroll and status.
pub fn draw(f: &mut Frame, app: &mut AppState) {
    if let Some(ref mut v) = app.viewer_screen {
        let area = f.area();
        v.area = area;
        let block = Block::default().borders(Borders::ALL).title(" ");
        let inner = block.inner(area);
        let content_height = inner.height.saturating_sub(1);
        let content_height_usize = content_height as usize;

        let (lines, total) = match v.view_mode {
            ViewerMode::Text => {
                let all = text_lines(v);
                let total = all.len();
                let visible: Vec<String> = all
                    .into_iter()
                    .skip(v.scroll)
                    .take(content_height_usize)
                    .collect();
                (visible, total)
            }
            ViewerMode::Hex => {
                let all = hex_lines(v);
                let total = all.len();
                let visible: Vec<String> = all
                    .into_iter()
                    .skip(v.scroll)
                    .take(content_height_usize)
                    .collect();
                (visible, total)
            }
        };

        let mode_label = match v.view_mode {
            ViewerMode::Text => "TEXT",
            ViewerMode::Hex => "HEX ",
        };
        let dark_bg = Color::Rgb(30, 30, 35);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} | {} | {} lines ", v.file_path, mode_label, total))
            .style(Style::default().bg(dark_bg).fg(Color::Cyan));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let content_height = inner.height.saturating_sub(1);
        let content_rect = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: content_height,
        };
        let content: String = lines.join("\n");
        let content_style = Style::default().bg(dark_bg).fg(Color::White);
        let para = Paragraph::new(content)
            .style(content_style)
            .wrap(Wrap { trim: false });
        f.render_widget(para, content_rect);

        let status = " Esc: close  H: toggle hex/text  ↑↓ PgUp/PgDn: scroll ";
        let status_rect = Rect {
            x: inner.x,
            y: inner.y + content_height,
            width: inner.width,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(status).style(content_style.fg(Color::DarkGray)),
            status_rect,
        );
    }
}
