//! File viewer (F3): view file as text or hex dump. ESC to close.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use std::sync::mpsc;

use crate::app_state::{AppState, ViewerMode, ViewerScreenState, ViewerState};
use crate::events::AppAction;
use crate::file_ops::FileOperations;
use crate::panel::PanelOperations;

/// Default bytes per line when width unknown; also minimum.
const HEX_BYTES_PER_LINE_DEFAULT: usize = 16;
const HEX_BYTES_PER_LINE_MAX: usize = 64;

/// Bytes per line to use so the hex dump fills the given width (address + hex + "  |" + ascii + "|").
/// Rounds to multiple of 8 for neat grouping; clamps to 8..=HEX_BYTES_PER_LINE_MAX.
fn hex_bytes_per_line_from_width(width: u16) -> usize {
    let w = width as usize;
    if w < 45 {
        return HEX_BYTES_PER_LINE_DEFAULT;
    }
    let bpl = (w - 13) / 4;
    let bpl = (bpl / 8).max(1) * 8;
    bpl.min(HEX_BYTES_PER_LINE_MAX).max(8)
}

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
                hex_cursor: 0,
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
/// Use full area width; we don't draw borders, so subtracting would leave right columns undrawn.
fn content_width(v: &ViewerScreenState) -> usize {
    let w = v.area.width;
    if w > 0 { w as usize } else { 80 }
}

/// Visible lines in viewer (area minus header and bottom bar).
fn visible_lines(v: &ViewerScreenState) -> usize {
    v.area.height.saturating_sub(2).max(1) as usize
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
                v.hex_cursor = 0;
                return Some(AppAction::Continue);
            }
            if v.view_mode == ViewerMode::Hex && !v.content.is_empty() {
                let content_rect = Rect {
                    x: 0,
                    y: 0,
                    width: v.area.width,
                    height: v.area.height.saturating_sub(2).max(1),
                };
                let chunks = Layout::horizontal([
                    Constraint::Percentage(70),
                    Constraint::Min(0),
                ])
                .split(content_rect);
                let bpl = hex_bpl_two_columns(chunks[0].width, chunks[1].width).max(1);
                let len = v.content.len();
                let total_lines = (len + bpl - 1) / bpl;
                let _max_scroll = total_lines.saturating_sub(height as usize).max(0);
                match key.code {
                    KeyCode::Left => v.hex_cursor = v.hex_cursor.saturating_sub(1),
                    KeyCode::Right => v.hex_cursor = (v.hex_cursor + 1).min(len.saturating_sub(1)),
                    KeyCode::Up => v.hex_cursor = v.hex_cursor.saturating_sub(bpl),
                    KeyCode::Down => v.hex_cursor = (v.hex_cursor + bpl).min(len.saturating_sub(1)),
                    KeyCode::PageUp => v.hex_cursor = v.hex_cursor.saturating_sub(bpl * height as usize),
                    KeyCode::PageDown => {
                        v.hex_cursor = (v.hex_cursor + bpl * height as usize).min(len.saturating_sub(1))
                    }
                    KeyCode::Home => v.hex_cursor = 0,
                    KeyCode::End => v.hex_cursor = len.saturating_sub(1),
                    _ => {}
                }
                // Keep scroll so the line containing hex_cursor is visible.
                let cursor_line = v.hex_cursor / bpl;
                if cursor_line < v.scroll {
                    v.scroll = cursor_line;
                } else if cursor_line >= v.scroll + height as usize {
                    v.scroll = cursor_line.saturating_sub(height as usize).saturating_add(1);
                }
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
        ViewerMode::Hex => {
            if v.area.width == 0 {
                return hex_line_count(v, 80);
            }
            let area = Rect {
                x: 0,
                y: 0,
                width: v.area.width,
                height: 1,
            };
            let chunks = Layout::horizontal([
                Constraint::Percentage(70),
                Constraint::Percentage(30),
            ])
            .split(area);
            hex_line_count_two_columns(v, chunks[0].width, chunks[1].width)
        }
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

/// Total hex lines (one per row). Single-column: from content_width; two-column: from left/right widths.
fn hex_line_count(v: &ViewerScreenState, content_width: u16) -> usize {
    let len = v.content.len();
    if len == 0 {
        return 0;
    }
    let bpl = hex_bytes_per_line_from_width(content_width);
    (len + bpl - 1) / bpl
}

/// Total hex lines when using two-column layout (responsive left/right).
fn hex_line_count_two_columns(v: &ViewerScreenState, left_width: u16, right_width: u16) -> usize {
    let len = v.content.len();
    if len == 0 {
        return 0;
    }
    let bpl = hex_bpl_two_columns(left_width, right_width);
    (len + bpl - 1) / bpl
}

/// Bytes per line for two-column layout: left (address+hex) and right (ASCII, no pipes).
fn hex_bpl_two_columns(left_width: u16, right_width: u16) -> usize {
    let l = left_width as usize;
    let r = right_width as usize;
    if l < 19 || r < 8 {
        return HEX_BYTES_PER_LINE_DEFAULT;
    }
    let bpl_left = (l - 10) / 3;
    let bpl_right = r;
    let bpl = bpl_left.min(bpl_right).max(8);
    let bpl = (bpl / 8).max(1) * 8;
    bpl.min(HEX_BYTES_PER_LINE_MAX)
}

/// Byte index i (in line) → start character index of its hex pair in the left line (after "AAAAAAAA: ").
fn hex_byte_column_in_line(i: usize) -> usize {
    (i / 8) * (8 * 3 + 2) + (i % 8) * 3
}

/// Two-column hex with optional cursor highlight: (left_lines, right_lines, total).
/// Left = address + hex; right = ascii. When cursor_byte is in the visible range, that byte is
/// highlighted in both columns (inverted style).
fn hex_visible_lines_two_columns_styled(
    v: &ViewerScreenState,
    scroll: usize,
    height: usize,
    left_width: u16,
    right_width: u16,
) -> (Vec<Line<'_>>, Vec<Line<'_>>, usize) {
    let bytes = &v.content;
    let cursor_byte = v.hex_cursor;
    if bytes.is_empty() {
        return (
            vec![Line::from("(empty file)")],
            vec![Line::from("")],
            0,
        );
    }
    let bpl = hex_bpl_two_columns(left_width, right_width);
    let total_lines = (bytes.len() + bpl - 1) / bpl;
    let scroll = scroll.min(total_lines.saturating_sub(1));
    let start_byte = scroll * bpl;
    let end_byte = ((scroll + height) * bpl).min(bytes.len());
    let mut left_lines = Vec::with_capacity(height.min(total_lines.saturating_sub(scroll)));
    let mut right_lines = Vec::with_capacity(left_lines.capacity());
    let addr_len = 10usize; // "AAAAAAAA: "
    let highlight_style = Style::default().bg(Color::DarkGray).fg(Color::White);
    let mut offset = start_byte;
    while offset < end_byte && left_lines.len() < height {
        let chunk = &bytes[offset..(offset + bpl).min(bytes.len())];
        let addr = format!("{:08x}: ", offset);
        let hex_part: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
        let hex_str = hex_part
            .chunks(8)
            .map(|c| c.join(" "))
            .collect::<Vec<_>>()
            .join("  ");
        let ascii: String = chunk
            .iter()
            .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
            .collect();
        let padding = bpl - chunk.len();
        let pad_hex = "   ".repeat(padding);
        let pad_ascii = " ".repeat(padding);
        let mut left_line_str = format!("{}{}{}", addr, hex_str, pad_hex);
        let mut right_line_str = format!("{}{}", ascii, pad_ascii);
        let lw = left_width as usize;
        let rw = right_width as usize;
        if left_line_str.len() > lw {
            left_line_str.truncate(lw);
        }
        if right_line_str.len() > rw {
            right_line_str.truncate(rw);
        }
        let cursor_in_this_line =
            cursor_byte >= offset && cursor_byte < offset + chunk.len();
        let local_cursor = cursor_byte.saturating_sub(offset);
        let left_line = if cursor_in_this_line && local_cursor < chunk.len() {
            let hex_start = addr_len + hex_byte_column_in_line(local_cursor);
            let hex_end = (hex_start + 2).min(left_line_str.len());
            let before = left_line_str.get(..hex_start).unwrap_or("").to_string();
            let sel = left_line_str.get(hex_start..hex_end).unwrap_or("").to_string();
            let after = left_line_str.get(hex_end..).unwrap_or("").to_string();
            Line::from(vec![
                Span::raw(before),
                Span::styled(sel, highlight_style),
                Span::raw(after),
            ])
        } else {
            Line::from(left_line_str.clone())
        };
        let right_line = if cursor_in_this_line && local_cursor < right_line_str.len() {
            let ch_start = local_cursor;
            let ch_end = (ch_start + 1).min(right_line_str.len());
            let before = right_line_str.get(..ch_start).unwrap_or("").to_string();
            let sel = right_line_str.get(ch_start..ch_end).unwrap_or("").to_string();
            let after = right_line_str.get(ch_end..).unwrap_or("").to_string();
            Line::from(vec![
                Span::raw(before),
                Span::styled(sel, highlight_style),
                Span::raw(after),
            ])
        } else {
            Line::from(right_line_str.clone())
        };
        left_lines.push(left_line);
        right_lines.push(right_line);
        offset += bpl;
    }
    if left_lines.is_empty() && start_byte < bytes.len() {
        left_lines.push(Line::from("00000000: (empty)"));
        right_lines.push(Line::from(""));
    }
    (left_lines, right_lines, total_lines)
}

/// Draw the viewer (text or hex) with scroll and status.
pub fn draw(f: &mut Frame, app: &mut AppState) {
    if let Some(state) = app.viewer_screen.as_mut() {
        let area = f.area();
        let dark_bg = Color::Rgb(30, 30, 35);
        let content_style = Style::default().bg(dark_bg).fg(Color::White);

        match state {
            ViewerState::Loading { file_path, .. } => {
                let header_rect = Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                };
                let content_rect = Rect {
                    x: area.x,
                    y: area.y + 1,
                    width: area.width,
                    height: area.height.saturating_sub(2),
                };
                let bottom_rect = Rect {
                    x: area.x,
                    y: area.y + area.height.saturating_sub(1),
                    width: area.width,
                    height: 1,
                };
                let header = Line::from(vec![
                    Span::styled(file_path.as_str(), Style::default().fg(Color::Cyan)),
                    Span::raw("  "),
                    Span::styled("Loading…", Style::default().fg(Color::DarkGray)),
                ]);
                f.render_widget(Paragraph::new(header).style(content_style), header_rect);
                let msg = "Reading file in background — Esc to close";
                f.render_widget(Paragraph::new(msg).style(content_style), content_rect);
                f.render_widget(
                    Paragraph::new(" Esc: close ").style(content_style.fg(Color::DarkGray)),
                    bottom_rect,
                );
            }
            ViewerState::Ready(v) => {
                v.area = area;
                let header_height = 1u16;
                let bottom_height = 1u16;
                let content_height = area.height.saturating_sub(header_height + bottom_height);
                let content_height_usize = content_height as usize;

                let header_rect = Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: header_height,
                };
                let content_rect = Rect {
                    x: area.x,
                    y: area.y + header_height,
                    width: area.width,
                    height: content_height,
                };
                let bottom_rect = Rect {
                    x: area.x,
                    y: area.y + area.height.saturating_sub(bottom_height),
                    width: area.width,
                    height: bottom_height,
                };

                let total = match v.view_mode {
                    ViewerMode::Text => {
                        let (lines, total) =
                            text_visible_lines_cached(v, v.scroll, content_height_usize);
                        let content: String = lines.join("\n");
                        let para = Paragraph::new(content)
                            .style(content_style)
                            .wrap(Wrap { trim: false });
                        f.render_widget(para, content_rect);
                        total
                    }
                    ViewerMode::Hex => {
                        // Use Min(0) for right column so it takes all remaining space;
                        // Percentage(70)+Percentage(30) can leave 1–2 columns undrawn when
                        // width doesn't divide evenly, leaving text-mode leftovers visible.
                        let chunks = Layout::horizontal([
                            Constraint::Percentage(70),
                            Constraint::Min(0),
                        ])
                        .split(content_rect);
                        let (left_lines, right_lines, total) = hex_visible_lines_two_columns_styled(
                            v,
                            v.scroll,
                            content_height_usize,
                            chunks[0].width,
                            chunks[1].width,
                        );
                        let left_text = ratatui::text::Text::from(left_lines);
                        let right_text = ratatui::text::Text::from(right_lines);
                        f.render_widget(
                            Paragraph::new(left_text)
                                .style(content_style)
                                .wrap(Wrap { trim: false }),
                            chunks[0],
                        );
                        f.render_widget(
                            Paragraph::new(right_text)
                                .style(content_style)
                                .wrap(Wrap { trim: false }),
                            chunks[1],
                        );
                        total
                    }
                };

                let mode_label = match v.view_mode {
                    ViewerMode::Text => "TEXT",
                    ViewerMode::Hex => "HEX",
                };
                let path_span = v.file_path.as_str();
                let right_info = format!("{} | {} lines", mode_label, total);
                let pad_len = (header_rect.width as usize)
                    .saturating_sub(path_span.len() + right_info.len())
                    .max(1);
                let header_line = Line::from(vec![
                    Span::styled(path_span, Style::default().fg(Color::Cyan)),
                    Span::raw(" ".repeat(pad_len)),
                    Span::styled(right_info, Style::default().fg(Color::DarkGray)),
                ]);
                f.render_widget(
                    Paragraph::new(header_line).style(content_style),
                    header_rect,
                );

                let bar = Line::from(vec![
                    Span::styled(" Esc ", content_style.fg(Color::DarkGray)),
                    Span::raw("close  "),
                    Span::styled(" H ", content_style.fg(Color::DarkGray)),
                    Span::raw("hex/text  "),
                    Span::styled(" ↑↓ ", content_style.fg(Color::DarkGray)),
                    Span::raw("PgUp/PgDn scroll"),
                ]);
                f.render_widget(Paragraph::new(bar).style(content_style.fg(Color::DarkGray)), bottom_rect);
            }
        }
    }
}
