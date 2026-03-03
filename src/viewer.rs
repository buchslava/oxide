//! File viewer (F3): view file as text or hex dump. ESC to close.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use std::sync::mpsc;

use crate::app_state::{AppState, ViewerMode, ViewerScreenState, ViewerState};
use crate::events::AppAction;
use crate::file_ops::FileOperations;
use crate::panel::PanelOperations;

const HEX_BYTES_PER_LINE: usize = 16;

/// Close the viewer and return to panels.
pub fn close_viewer(app: &mut AppState) {
    app.viewer_screen = None;
}

/// Open the currently selected file in the viewer. Reads file in a background thread so Esc works immediately for large files.
/// Returns true if the viewer was opened (shows "Loading..." until read completes).
pub fn open_viewer(app: &mut AppState) -> bool {
    let cwd = app.get_current_dir().to_string();
    if let Some(file) = app.active_panel_mut().get_selected_file() {
        if !file.is_dir && !file.is_parent_dir() {
            let path = FileOperations::join_path(&cwd, &file.name);
            let file_path_str = path.to_string_lossy().to_string();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(std::fs::read(&path));
            });
            app.viewer_screen = Some(ViewerState::Loading {
                file_path: file_path_str,
                rx,
            });
            return true;
        }
    }
    false
}

/// Poll the viewer loading channel; when the background read completes, replace Loading with Ready (or close on error).
/// Call from the main loop so content appears without blocking Esc.
/// Returns true if the state changed (caller may redraw).
pub fn poll_viewer_loading(app: &mut AppState) -> bool {
    let rx = match &mut app.viewer_screen {
        Some(ViewerState::Loading { rx, .. }) => rx,
        _ => return false,
    };
    match rx.try_recv() {
        Ok(Ok(content)) => {
            let file_path = match std::mem::take(&mut app.viewer_screen) {
                Some(ViewerState::Loading { file_path, .. }) => file_path,
                _ => return false,
            };
            app.viewer_screen = Some(ViewerState::Ready(ViewerScreenState {
                file_path,
                content,
                view_mode: ViewerMode::Text,
                scroll: 0,
                area: Rect::default(),
                text_line_starts: None,
                text_display_cumulative: None,
                text_cache_width: 0,
            }));
            true
        }
        Ok(Err(_)) => {
            app.viewer_screen = None;
            true
        }
        Err(mpsc::TryRecvError::Empty) => false,
        Err(mpsc::TryRecvError::Disconnected) => {
            app.viewer_screen = None;
            true
        }
    }
}

/// Content width in characters (from last draw; fallback when not yet drawn).
fn content_width(v: &ViewerScreenState) -> usize {
    let w = v.area.width.saturating_sub(2); // borders
    if w > 0 { w as usize } else { 80 }
}

/// Visible lines in viewer (area minus block borders and status line).
fn visible_lines(v: &ViewerScreenState) -> usize {
    v.area.height.saturating_sub(3).max(1) as usize
}

/// Split a single line into display lines of at most `width` chars (wrap long lines).
fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut s = line;
    while !s.is_empty() {
        let n = s.chars().take(width).count();
        let (chunk, rest) = if n < s.chars().count() {
            let idx = s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len());
            s.split_at(idx)
        } else {
            (s, "")
        };
        out.push(chunk.to_string());
        s = rest;
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Handle key when viewer is open. Returns Some(action) when handled, None if not in viewer.
/// ESC closes immediately (also when file is still loading); also accept raw 0x1b.
pub fn handle_viewer_key(app: &mut AppState, key: crossterm::event::KeyEvent) -> Option<AppAction> {
    use crossterm::event::KeyCode;
    match app.viewer_screen.as_mut()? {
        ViewerState::Loading { .. } => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
                return Some(AppAction::ViewerClose);
            }
            return Some(AppAction::Continue);
        }
        ViewerState::Ready(v) => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
                return Some(AppAction::ViewerClose);
            }
            let height = visible_lines(v);
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
                KeyCode::Up => v.scroll = v.scroll.saturating_sub(1),
                KeyCode::Down => v.scroll = (v.scroll + 1).min(max_scroll),
                KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(height),
                KeyCode::PageDown => v.scroll = (v.scroll + height).min(max_scroll),
                KeyCode::Home => v.scroll = 0,
                KeyCode::End => v.scroll = max_scroll,
                _ => {}
            }
            Some(AppAction::Continue)
        }
    }
}

fn line_count(v: &mut ViewerScreenState) -> usize {
    match v.view_mode {
        ViewerMode::Text => {
            ensure_text_cache(v);
            text_line_count_cached(v)
        }
        ViewerMode::Hex => hex_line_count(v),
    }
}

/// Characters safe to show in text mode (avoids binary/control chars that corrupt the terminal).
fn safe_text_char(c: char) -> bool {
    c == '\t' || c == '\n' || c == '\r' || (c.is_ascii() && c >= ' ' && c <= '~')
}

/// Replace non-printable and binary characters with '.' so the terminal display is not corrupted.
/// Keeps tab, newline, carriage return and printable ASCII (0x20–0x7E); same convention as hex dump ASCII column.
fn sanitize_text_for_display(s: &str) -> String {
    s.chars().map(|c| if safe_text_char(c) { c } else { '.' }).collect()
}

/// Logical (file) lines, sanitized for display.
fn text_lines_logical(v: &ViewerScreenState) -> Vec<String> {
    let s = String::from_utf8_lossy(&v.content);
    let sanitized = sanitize_text_for_display(&s);
    let mut lines: Vec<String> = sanitized.lines().map(|l| l.to_string()).collect();
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

/// Display lines for text mode: long lines wrapped at content width (like less default).
fn text_lines_wrapped(v: &ViewerScreenState) -> Vec<String> {
    let logical = text_lines_logical(v);
    let width = content_width(v);
    logical.iter().flat_map(|l| wrap_line(l, width)).collect()
}

/// Build or refresh text cache (line starts + cumulative display count) so paging is O(1)/O(log n) like MC.
fn ensure_text_cache(v: &mut ViewerScreenState) {
    let width = content_width(v) as usize;
    if width == 0 {
        return;
    }
    if v.text_display_cumulative.is_some() && v.text_cache_width == width as u16 {
        return;
    }
    let content = &v.content;
    let mut line_starts = Vec::new();
    line_starts.push(0);
    for (i, &b) in content.iter().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    if content.last() != Some(&b'\n') && !content.is_empty() {
        line_starts.push(content.len());
    }
    let mut cumulative = Vec::with_capacity(line_starts.len().saturating_sub(1));
    let mut cum = 0usize;
    for i in 0..line_starts.len().saturating_sub(1) {
        let start = line_starts[i];
        let end = *line_starts.get(i + 1).unwrap_or(&content.len());
        let line_bytes = &content[start..end];
        let s = sanitize_text_for_display(&String::from_utf8_lossy(line_bytes));
        cum += wrap_line(&s, width).len();
        cumulative.push(cum);
    }
    v.text_line_starts = Some(line_starts);
    v.text_display_cumulative = Some(cumulative);
    v.text_cache_width = width as u16;
}

/// Total display lines in text mode using cache (no full wrap).
fn text_line_count_cached(v: &ViewerScreenState) -> usize {
    v.text_display_cumulative
        .as_ref()
        .and_then(|c| c.last().copied())
        .unwrap_or_else(|| text_lines_wrapped(v).len())
}

/// Visible display lines for text mode: only wrap the window that fits on screen (MC-style fast paging).
fn text_visible_lines_cached(v: &mut ViewerScreenState, scroll: usize, height: usize) -> (Vec<String>, usize) {
    ensure_text_cache(v);
    let line_starts = match &v.text_line_starts {
        Some(s) => s,
        None => {
            let all = text_lines_wrapped(v);
            let total = all.len();
            let visible: Vec<String> = all.into_iter().skip(scroll).take(height).collect();
            return (visible, total);
        }
    };
    let cumulative = match &v.text_display_cumulative {
        Some(c) => c,
        None => {
            let all = text_lines_wrapped(v);
            let total = all.len();
            let visible: Vec<String> = all.into_iter().skip(scroll).take(height).collect();
            return (visible, total);
        }
    };
    let total = cumulative.last().copied().unwrap_or(0);
    if total == 0 {
        return (vec![], 0);
    }
    let scroll = scroll.min(total.saturating_sub(1));
    let width = content_width(v);
    let num_logical = line_starts.len().saturating_sub(1);
    if num_logical == 0 {
        return (vec![], total);
    }
    let prev_cum = |i: usize| if i == 0 { 0 } else { cumulative.get(i - 1).copied().unwrap_or(0) };
    let (logical_line, segment_in_line) = match cumulative.binary_search(&scroll) {
        Ok(i) => (i + 1, 0),           // scroll at end of line i → start at line i+1, segment 0
        Err(i) => (i, scroll - prev_cum(i)), // scroll inside line i
    };
    let logical_line = logical_line.min(num_logical.saturating_sub(1));
    let content = &v.content;
    let mut out = Vec::with_capacity(height);
    let mut line_idx = logical_line;
    let mut segment_skip = segment_in_line;
    while out.len() < height && line_idx < num_logical {
        let start = line_starts[line_idx];
        let end = line_starts.get(line_idx + 1).copied().unwrap_or(content.len());
        let line_bytes = &content[start..end];
        let s = sanitize_text_for_display(&String::from_utf8_lossy(line_bytes));
        let wrapped = wrap_line(&s, width);
        for (_seg_idx, seg) in wrapped.into_iter().enumerate() {
            if segment_skip > 0 {
                segment_skip -= 1;
                continue;
            }
            out.push(seg);
            if out.len() >= height {
                break;
            }
        }
        line_idx += 1;
        segment_skip = 0;
    }
    (out, total)
}

/// Total hex lines (one per HEX_BYTES_PER_LINE bytes). O(1), no formatting.
fn hex_line_count(v: &ViewerScreenState) -> usize {
    let len = v.content.len();
    if len == 0 {
        return 0;
    }
    (len + HEX_BYTES_PER_LINE - 1) / HEX_BYTES_PER_LINE
}

/// Format only the visible window of hex lines (lazy: no full-file iteration).
fn hex_visible_lines(v: &ViewerScreenState, scroll: usize, height: usize) -> Vec<String> {
    let bytes = &v.content;
    if bytes.is_empty() {
        return vec!["(empty file)".to_string()];
    }
    let total_lines = hex_line_count(v);
    let scroll = scroll.min(total_lines.saturating_sub(1));
    let start_byte = scroll * HEX_BYTES_PER_LINE;
    let end_byte = ((scroll + height) * HEX_BYTES_PER_LINE).min(bytes.len());
    let mut lines = Vec::with_capacity(height.min(total_lines.saturating_sub(scroll)));
    let mut offset = start_byte;
    while offset < end_byte && lines.len() < height {
        let chunk = &bytes[offset..(offset + HEX_BYTES_PER_LINE).min(bytes.len())];
        let addr = format!("{:08x}: ", offset);
        let hex_part: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
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
        offset += HEX_BYTES_PER_LINE;
    }
    if lines.is_empty() && start_byte < bytes.len() {
        lines.push("00000000: (empty)".to_string());
    }
    lines
}

/// Draw the viewer (text or hex) with scroll and status.
pub fn draw(f: &mut Frame, app: &mut AppState) {
    if let Some(state) = app.viewer_screen.as_mut() {
        let area = f.area();
        let dark_bg = Color::Rgb(30, 30, 35);
        let content_style = Style::default().bg(dark_bg).fg(Color::White);

        match state {
            ViewerState::Loading { file_path, .. } => {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} | Loading… ", file_path))
                    .style(Style::default().bg(dark_bg).fg(Color::Cyan));
                let inner = block.inner(area);
                f.render_widget(block, area);
                let msg = "Reading file in background — Esc to close";
                let para = Paragraph::new(msg).style(content_style);
                f.render_widget(para, inner);
                let status = " Esc: close ";
                let status_rect = Rect {
                    x: inner.x,
                    y: inner.y + inner.height.saturating_sub(1),
                    width: inner.width,
                    height: 1,
                };
                f.render_widget(
                    Paragraph::new(status).style(content_style.fg(Color::DarkGray)),
                    status_rect,
                );
            }
            ViewerState::Ready(v) => {
                v.area = area;
                let block = Block::default().borders(Borders::ALL).title(" ");
                let inner = block.inner(area);
                let content_height = inner.height.saturating_sub(1);
                let content_height_usize = content_height as usize;

                let (lines, total) = match v.view_mode {
                    ViewerMode::Text => {
                        text_visible_lines_cached(v, v.scroll, content_height_usize)
                    }
                    ViewerMode::Hex => {
                        let total = hex_line_count(v);
                        let visible = hex_visible_lines(v, v.scroll, content_height_usize);
                        (visible, total)
                    }
                };

                let mode_label = match v.view_mode {
                    ViewerMode::Text => "TEXT",
                    ViewerMode::Hex => "HEX ",
                };
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
    }
}
