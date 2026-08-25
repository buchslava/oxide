//! File viewer (F3): view file as text, hex dump, or rendered markdown. ESC to close.

use std::fmt::Write;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

use crate::util;

use crate::app::state::AppState;
use crate::browser::viewer_image::{
    collect_raster_view_entries, draw_image_loading, draw_image_ready, finish_image_loading,
    handle_image_loading_key, handle_image_loading_mouse, handle_image_ready_key,
    handle_image_ready_mouse, single_fs_raster_entry, spawn_image_load_thread, ImageLoadMsg,
    ImageLoadingState, ImageViewerState,
};
use crate::browser::viewer_markdown::{
    clear_markdown_nav, draw_markdown, handle_markdown_key, handle_markdown_mouse,
    is_markdown_path, on_markdown_ready, text_viewer_to_markdown, try_open_markdown_from_bytes,
    MarkdownViewerState,
};
use crate::core::text_format::wrap_line;

/// Building the text line index scans the whole file; keep text mode off above this size (hex stays responsive).
pub const MAX_TEXT_VIEW_BYTES: usize = 64 * 1024 * 1024;

/// Viewer is either loading file in background (Esc still closes) or ready with content.
pub enum ViewerState {
    /// File is being read on a background thread; Esc sets `cancel` and closes without waiting.
    Loading {
        file_path: String,
        rx: mpsc::Receiver<io::Result<Vec<u8>>>,
        /// When set (e.g. from Find file content search), scroll to this 1-based line when ready.
        initial_line: Option<u64>,
        cancel: Arc<AtomicBool>,
    },
    /// Content loaded; normal view.
    Ready(ViewerScreenState),
    /// Raster images (PNG/JPEG/GIF): load bytes in background, then show with ratatui-image.
    ImageLoading(ImageLoadingState),
    /// Multi-tab image view.
    ImageReady(ImageViewerState),
    /// Markdown documents (.md / .markdown / .mdx): rendered preview.
    MarkdownReady(MarkdownViewerState),
}

/// State when the file viewer content is ready (F3). Text and hex modes.
pub struct ViewerScreenState {
    pub file_path: String,
    /// Raw file bytes (for hex mode; text mode uses same buffer decoded).
    pub content: Vec<u8>,
    /// Display mode: text (lines) or hex dump.
    pub view_mode: ViewerMode,
    /// First visible line index (scroll offset).
    pub scroll: usize,
    /// Hex mode: byte offset of the current character (highlighted in hex and ASCII columns).
    pub hex_cursor: usize,
    /// Last draw area (for consistent layout).
    pub area: Rect,
    /// Text mode: byte offset of start of each logical line (len = num_lines+1). Used for fast paging (MC-style).
    pub text_line_starts: Option<Vec<usize>>,
    /// Text mode: cumulative display line count after each logical line. cumulative[i] = total display lines for logical lines 0..=i.
    pub text_display_cumulative: Option<Vec<usize>>,
    /// Text mode: content width (chars) this cache was built for; 0 = invalid.
    pub text_cache_width: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerMode {
    Text,
    Hex,
}
use crate::app::events::AppAction;
use crate::browser::panel::PanelOperations;
use crate::core::file_ops::FileOperations;
use crate::core::location::PanelLocation;
use crate::core::panel_backend;
use crate::ui::theme::ViewerPalette;

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
    match app.viewer_screen.as_ref() {
        Some(ViewerState::Loading { cancel, .. })
        | Some(ViewerState::ImageLoading(ImageLoadingState { cancel, .. })) => {
            cancel.store(true, Ordering::Relaxed);
        }
        _ => {}
    }
    app.viewer_screen = None;
    clear_markdown_nav(app);
}

/// Open the currently selected file in the viewer. Reads file in a background thread so Esc works immediately for large files.
/// Raster images (PNG/JPEG/GIF) open in the graphical image viewer (possibly multiple tabs when files are marked).
/// Works for both filesystem and files inside ZIP (uses panel_backend::read_file).
/// Returns true if the viewer was opened (shows "Loading..." until read completes).
pub fn open_viewer(app: &mut AppState) -> bool {
    if !app.markdown_viewer_follow_link {
        clear_markdown_nav(app);
    }
    app.markdown_viewer_follow_link = false;
    let loc = app.get_current_location();
    let panel = app.active_panel_ref();
    if let Some((entries, start)) = collect_raster_view_entries(panel, &loc) {
        if !entries.is_empty() {
            let n = entries.len();
            let cancel = Arc::new(AtomicBool::new(false));
            let cancel_thread = Arc::clone(&cancel);
            let (tx, rx) = mpsc::channel();
            spawn_image_load_thread(entries.clone(), cancel_thread, tx);
            app.viewer_screen = Some(ViewerState::ImageLoading(
                ImageLoadingState {
                    entries,
                    current: start,
                    buffers: vec![None; n],
                    loaded: 0,
                    rx,
                    cancel,
                },
            ));
            return true;
        }
    }
    if let Some(file) = app.active_panel_mut().get_selected_file() {
        if !file.is_dir && !file.is_parent_dir() {
            let file_path_str = panel_backend::join_path_display(&loc, &file.name);
            let loc_clone = loc.clone();
            let name = file.name.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let cancel_thread = Arc::clone(&cancel);
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let result = match &loc_clone {
                    PanelLocation::Fs(p) => {
                        let path = FileOperations::join_path(p, &name);
                        util::read_path_chunked(&path, &cancel_thread)
                    }
                    _ => {
                        if cancel_thread.load(Ordering::Relaxed) {
                            Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "viewer load cancelled",
                            ))
                        } else {
                            panel_backend::read_file(&loc_clone, &name)
                        }
                    }
                };
                let _ = tx.send(result);
            });
            app.viewer_screen = Some(ViewerState::Loading {
                file_path: file_path_str,
                rx,
                initial_line: None,
                cancel,
            });
            return true;
        }
    }
    false
}

/// Open a file by path in the viewer (e.g. from Find file results). Optionally scroll to 1-based line.
pub fn open_viewer_path(
    app: &mut AppState,
    path: std::path::PathBuf,
    line: Option<u64>,
) -> bool {
    if !app.markdown_viewer_follow_link {
        clear_markdown_nav(app);
    }
    app.markdown_viewer_follow_link = false;
    if let Some((entries, start)) = single_fs_raster_entry(path.clone()) {
        if !entries.is_empty() {
            let n = entries.len();
            let cancel = Arc::new(AtomicBool::new(false));
            let cancel_thread = Arc::clone(&cancel);
            let (tx, rx) = mpsc::channel();
            spawn_image_load_thread(entries.clone(), cancel_thread, tx);
            app.viewer_screen = Some(ViewerState::ImageLoading(
                ImageLoadingState {
                    entries,
                    current: start,
                    buffers: vec![None; n],
                    loaded: 0,
                    rx,
                    cancel,
                },
            ));
            return true;
        }
    }
    let path_clone = path.clone();
    let file_path_str = path.display().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = Arc::clone(&cancel);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = util::read_path_chunked(&path_clone, &cancel_thread);
        let _ = tx.send(result);
    });
    app.viewer_screen = Some(ViewerState::Loading {
        file_path: file_path_str,
        rx,
        initial_line: line,
        cancel,
    });
    true
}

/// Poll the viewer loading channel; when the background read completes, replace Loading with Ready (or close on error).
/// Call from the main loop so content appears without blocking Esc.
/// Returns true if the state changed (caller may redraw).
pub fn poll_viewer_loading(app: &mut AppState) -> bool {
    if let Some(ViewerState::ImageLoading(loading)) = app.viewer_screen.as_mut() {
        let mut progressed = false;
        while let Ok(msg) = loading.rx.try_recv() {
            progressed = true;
            match msg {
                ImageLoadMsg::Part { index, bytes } => {
                    if index < loading.buffers.len() && loading.buffers[index].is_none() {
                        loading.buffers[index] = Some(bytes);
                        loading.loaded += 1;
                    }
                }
                ImageLoadMsg::Failed(e) => {
                    app.set_timed_toast_alert(
                        std::time::Duration::from_secs(4),
                        format!("Image read: {e}"),
                    );
                    app.viewer_screen = None;
                    return true;
                }
            }
        }
        let complete = if let Some(ViewerState::ImageLoading(loading)) = app.viewer_screen.as_ref()
        {
            !loading.buffers.is_empty() && loading.loaded >= loading.buffers.len()
        } else {
            false
        };
        if complete {
            let taken = app.viewer_screen.take();
            if let Some(ViewerState::ImageLoading(ld)) = taken {
                if let Some(ready) = finish_image_loading(ld, app) {
                    app.viewer_screen = Some(ViewerState::ImageReady(ready));
                }
            }
            return true;
        }
        return progressed;
    }

    let rx = match &mut app.viewer_screen {
        Some(ViewerState::Loading { rx, .. }) => rx,
        _ => return false,
    };
    match rx.try_recv() {
        Ok(Ok(content)) => {
            let (file_path, initial_line) = match std::mem::take(&mut app.viewer_screen) {
                Some(ViewerState::Loading {
                    file_path,
                    initial_line,
                    ..
                }) => (file_path, initial_line),
                _ => return false,
            };
            let scroll = initial_line
                .map(|l| (l as usize).saturating_sub(1))
                .unwrap_or(0);
            if let Some(mut md) = try_open_markdown_from_bytes(&file_path, &content) {
                on_markdown_ready(&mut md, app);
                app.viewer_screen = Some(ViewerState::MarkdownReady(md));
                return true;
            }
            let view_mode = if content.len() > MAX_TEXT_VIEW_BYTES {
                ViewerMode::Hex
            } else {
                ViewerMode::Text
            };
            app.viewer_screen = Some(ViewerState::Ready(ViewerScreenState {
                file_path,
                content,
                view_mode,
                scroll,
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
    if w > 0 {
        w as usize
    } else {
        80
    }
}

/// Visible lines in viewer (area minus header and bottom bar).
fn visible_lines(v: &ViewerScreenState) -> usize {
    v.area.height.saturating_sub(2).max(1) as usize
}

/// Handle key when viewer is open. Returns Some(action) when handled, None if not in viewer.
/// ESC closes immediately (also when file is still loading); also accept raw 0x1b.
pub fn handle_viewer_key(
    app: &mut AppState,
    key: KeyEvent,
) -> Option<AppAction> {
    match app.viewer_screen.as_mut()? {
        ViewerState::ImageLoading(..) => {
            handle_image_loading_key(key.code).or(Some(AppAction::Continue))
        }
        ViewerState::ImageReady(img) => handle_image_ready_key(img, key),
        ViewerState::MarkdownReady(..) => handle_markdown_key(app, key),
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
            if is_markdown_path(&v.file_path)
                && v.view_mode == ViewerMode::Text
                && (key.code == KeyCode::Char('t') || key.code == KeyCode::Char('T'))
            {
                let taken = app.viewer_screen.take();
                if let Some(ViewerState::Ready(v)) = taken {
                    if let Some(md) = text_viewer_to_markdown(v) {
                        app.viewer_screen = Some(ViewerState::MarkdownReady(md));
                    }
                }
                return Some(AppAction::Continue);
            }
            let height = visible_lines(v);
            if key.code == KeyCode::Char('h') || key.code == KeyCode::Char('H') {
                match v.view_mode {
                    ViewerMode::Text => {
                        v.view_mode = ViewerMode::Hex;
                        v.scroll = 0;
                        v.hex_cursor = 0;
                    }
                    ViewerMode::Hex => {
                        if v.content.len() > MAX_TEXT_VIEW_BYTES {
                            app.set_timed_toast_alert(
                                std::time::Duration::from_secs(4),
                                "Text mode needs a full-file index; use hex for files over 64 MB.",
                            );
                        } else {
                            v.view_mode = ViewerMode::Text;
                            v.scroll = 0;
                            v.hex_cursor = 0;
                        }
                    }
                }
                return Some(AppAction::Continue);
            }
            if v.view_mode == ViewerMode::Hex && !v.content.is_empty() {
                let content_rect = Rect {
                    x: 0,
                    y: 0,
                    width: v.area.width,
                    height: v.area.height.saturating_sub(2).max(1),
                };
                let chunks = Layout::horizontal([Constraint::Percentage(70), Constraint::Min(0)])
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
                    KeyCode::PageUp => {
                        v.hex_cursor = v.hex_cursor.saturating_sub(bpl * height as usize)
                    }
                    KeyCode::PageDown => {
                        v.hex_cursor =
                            (v.hex_cursor + bpl * height as usize).min(len.saturating_sub(1))
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
                    v.scroll = cursor_line
                        .saturating_sub(height as usize)
                        .saturating_add(1);
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

const VIEWER_MOUSE_SCROLL_LINES: usize = 3;

fn apply_viewer_scroll_wheel(
    v: &mut ViewerScreenState,
    delta_display_lines: isize,
) {
    let height = visible_lines(v);
    if v.view_mode == ViewerMode::Hex && !v.content.is_empty() {
        let content_rect = Rect {
            x: 0,
            y: 0,
            width: v.area.width,
            height: v.area.height.saturating_sub(2).max(1),
        };
        let chunks = Layout::horizontal([Constraint::Percentage(70), Constraint::Min(0)])
            .split(content_rect);
        let bpl = hex_bpl_two_columns(chunks[0].width, chunks[1].width).max(1);
        let len = v.content.len();
        let total_lines = (len + bpl - 1) / bpl;
        let max_scroll = total_lines.saturating_sub(height as usize).max(0);
        let s = v.scroll as isize + delta_display_lines;
        v.scroll = s.clamp(0, max_scroll as isize) as usize;
        return;
    }
    let total_lines = line_count(v);
    let max_scroll = total_lines.saturating_sub(height).max(0);
    let s = v.scroll as isize + delta_display_lines;
    v.scroll = s.clamp(0, max_scroll as isize) as usize;
}

/// Capture mouse while the viewer is open; wheel scrolls text/hex like ↑↓ (see [`VIEWER_MOUSE_SCROLL_LINES`]).
pub fn handle_viewer_mouse(
    app: &mut AppState,
    mouse_event: MouseEvent,
) -> bool {
    let Some(state) = app.viewer_screen.as_mut() else {
        return false;
    };
    match state {
        ViewerState::ImageLoading(..) => handle_image_loading_mouse(&mouse_event),
        ViewerState::ImageReady(img) => handle_image_ready_mouse(img, mouse_event),
        ViewerState::MarkdownReady(_) => handle_markdown_mouse(app, mouse_event),
        ViewerState::Loading { .. } => true,
        ViewerState::Ready(v) => {
            let n = VIEWER_MOUSE_SCROLL_LINES as isize;
            match mouse_event.kind {
                MouseEventKind::ScrollUp => apply_viewer_scroll_wheel(v, -n),
                MouseEventKind::ScrollDown => apply_viewer_scroll_wheel(v, n),
                _ => {}
            }
            true
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
            let chunks =
                Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)])
                    .split(area);
            hex_line_count_two_columns(v, chunks[0].width, chunks[1].width)
        }
    }
}

/// Characters safe to show in text mode (avoids binary/control chars that corrupt the terminal).
fn safe_text_char(c: char) -> bool {
    c == '\n' || (c.is_ascii() && c >= ' ' && c <= '~')
}

/// Replace non-printable and binary characters with '.' so the terminal display is not corrupted.
/// Keeps tab, newline and printable ASCII (0x20–0x7E); same convention as hex dump ASCII column.
/// Carriage returns are stripped to avoid terminal redraw artifacts on CRLF files.
fn sanitize_text_for_display(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\r' => {}
            '\t' => out.push_str("    "),
            '\n' => out.push('\n'),
            _ if safe_text_char(c) => out.push(c),
            _ => out.push('.'),
        }
    }
    out
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

/// Convert one logical line byte-slice to a clean display string:
/// strip trailing CR/LF separators, then sanitize control/binary chars.
fn sanitize_line_bytes_for_display(line_bytes: &[u8]) -> String {
    let mut raw = String::from_utf8_lossy(line_bytes).to_string();
    while raw.ends_with('\n') || raw.ends_with('\r') {
        raw.pop();
    }
    sanitize_text_for_display(&raw)
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
        let s = sanitize_line_bytes_for_display(line_bytes);
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
fn text_visible_lines_cached(
    v: &mut ViewerScreenState,
    scroll: usize,
    height: usize,
) -> (Vec<String>, usize) {
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
    let prev_cum = |i: usize| {
        if i == 0 {
            0
        } else {
            cumulative.get(i - 1).copied().unwrap_or(0)
        }
    };
    let (logical_line, segment_in_line) = match cumulative.binary_search(&scroll) {
        Ok(i) => (i + 1, 0), // scroll at end of line i → start at line i+1, segment 0
        Err(i) => (i, scroll - prev_cum(i)), // scroll inside line i
    };
    let logical_line = logical_line.min(num_logical.saturating_sub(1));
    let content = &v.content;
    let mut out = Vec::with_capacity(height);
    let mut line_idx = logical_line;
    let mut segment_skip = segment_in_line;
    while out.len() < height && line_idx < num_logical {
        let start = line_starts[line_idx];
        let end = line_starts
            .get(line_idx + 1)
            .copied()
            .unwrap_or(content.len());
        let line_bytes = &content[start..end];
        let s = sanitize_line_bytes_for_display(line_bytes);
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
fn hex_line_count(
    v: &ViewerScreenState,
    content_width: u16,
) -> usize {
    let len = v.content.len();
    if len == 0 {
        return 0;
    }
    let bpl = hex_bytes_per_line_from_width(content_width);
    (len + bpl - 1) / bpl
}

/// Total hex lines when using two-column layout (responsive left/right).
fn hex_line_count_two_columns(
    v: &ViewerScreenState,
    left_width: u16,
    right_width: u16,
) -> usize {
    let len = v.content.len();
    if len == 0 {
        return 0;
    }
    let bpl = hex_bpl_two_columns(left_width, right_width);
    (len + bpl - 1) / bpl
}

/// Bytes per line for two-column layout: left (address+hex) and right (ASCII, no pipes).
fn hex_bpl_two_columns(
    left_width: u16,
    right_width: u16,
) -> usize {
    let left_cols = left_width as usize;
    let right_cols = right_width as usize;
    if left_cols < 19 || right_cols < 8 {
        return HEX_BYTES_PER_LINE_DEFAULT;
    }
    let bpl_left = (left_cols - 10) / 3;
    let bpl_right = right_cols;
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
    hex_cursor_highlight: Style,
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
    let highlight_style = hex_cursor_highlight;
    let mut offset = start_byte;
    while offset < end_byte && left_lines.len() < height {
        let chunk = &bytes[offset..(offset + bpl).min(bytes.len())];
        let mut hex_str = String::with_capacity(chunk.len() * 3 + chunk.len() / 8 + 8);
        for (i, b) in chunk.iter().enumerate() {
            if i > 0 {
                hex_str.push(' ');
                if i % 8 == 0 {
                    hex_str.push(' ');
                }
            }
            let _ = write!(hex_str, "{:02x}", b);
        }
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        let padding = bpl - chunk.len();
        let mut left_line_str = String::with_capacity(addr_len + hex_str.len() + padding * 3);
        let _ = write!(
            left_line_str,
            "{:08x}: {}",
            offset, hex_str
        );
        for _ in 0..padding {
            left_line_str.push_str("   ");
        }
        let mut right_line_str = ascii;
        right_line_str.extend(std::iter::repeat(' ').take(padding));
        let lw = left_width as usize;
        let rw = right_width as usize;
        if left_line_str.len() > lw {
            left_line_str.truncate(lw);
        }
        if right_line_str.len() > rw {
            right_line_str.truncate(rw);
        }
        let cursor_in_this_line = cursor_byte >= offset && cursor_byte < offset + chunk.len();
        let local_cursor = cursor_byte.saturating_sub(offset);
        let left_line = if cursor_in_this_line && local_cursor < chunk.len() {
            let hex_start = addr_len + hex_byte_column_in_line(local_cursor);
            let hex_end = (hex_start + 2).min(left_line_str.len());
            let before = left_line_str.get(..hex_start).unwrap_or("").to_string();
            let sel = left_line_str
                .get(hex_start..hex_end)
                .unwrap_or("")
                .to_string();
            let after = left_line_str.get(hex_end..).unwrap_or("").to_string();
            Line::from(vec![
                Span::raw(before),
                Span::styled(sel, highlight_style),
                Span::raw(after),
            ])
        } else {
            Line::from(left_line_str)
        };
        let right_line = if cursor_in_this_line && local_cursor < right_line_str.len() {
            let ch_start = local_cursor;
            let ch_end = (ch_start + 1).min(right_line_str.len());
            let before = right_line_str.get(..ch_start).unwrap_or("").to_string();
            let sel = right_line_str
                .get(ch_start..ch_end)
                .unwrap_or("")
                .to_string();
            let after = right_line_str.get(ch_end..).unwrap_or("").to_string();
            Line::from(vec![
                Span::raw(before),
                Span::styled(sel, highlight_style),
                Span::raw(after),
            ])
        } else {
            Line::from(right_line_str)
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
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    let Some(mut state) = app.viewer_screen.take() else {
        return;
    };
    let area = f.area();
    let vp: ViewerPalette = app.ui_palette.viewer;

    let content_style = Style::default().bg(vp.background).fg(vp.text);

    match &mut state {
        ViewerState::ImageLoading(ld) => {
            draw_image_loading(f, ld, vp, app.ui_palette.dialog);
        }
        ViewerState::ImageReady(img) => {
            draw_image_ready(f, img, app, vp);
        }
        ViewerState::MarkdownReady(md) => {
            draw_markdown(f, md, app, vp);
        }
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
                Span::styled(
                    file_path.as_str(),
                    Style::default().fg(vp.header_path),
                ),
                Span::raw("  "),
                Span::styled("Loading…", Style::default().fg(vp.muted)),
            ]);
            f.render_widget(
                Paragraph::new(header).style(content_style),
                header_rect,
            );
            let msg = "Reading file in background — Esc to close";
            f.render_widget(
                Paragraph::new(msg).style(content_style),
                content_rect,
            );
            f.render_widget(
                Paragraph::new(" Esc: close ").style(content_style.fg(vp.muted)),
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

            // Clear the whole content area each frame to prevent stale glyphs when
            // new page has fewer/shorter lines than the previous one.
            f.render_widget(
                Block::default().style(content_style),
                content_rect,
            );

            let total = match v.view_mode {
                ViewerMode::Text => {
                    let (lines, total) =
                        text_visible_lines_cached(v, v.scroll, content_height_usize);
                    let text_lines: Vec<Line> = lines.into_iter().map(Line::from).collect();
                    let para = Paragraph::new(Text::from(text_lines)).style(content_style);
                    f.render_widget(para, content_rect);
                    total
                }
                ViewerMode::Hex => {
                    // Use Min(0) for right column so it takes all remaining space;
                    // Percentage(70)+Percentage(30) can leave 1–2 columns undrawn when
                    // width doesn't divide evenly, leaving text-mode leftovers visible.
                    let chunks =
                        Layout::horizontal([Constraint::Percentage(70), Constraint::Min(0)])
                            .split(content_rect);
                    let (left_lines, right_lines, total) = hex_visible_lines_two_columns_styled(
                        v,
                        v.scroll,
                        content_height_usize,
                        chunks[0].width,
                        chunks[1].width,
                        vp.hex_cursor_highlight_style(),
                    );
                    let left_text = Text::from(left_lines);
                    let right_text = Text::from(right_lines);
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
                Span::styled(
                    path_span,
                    Style::default().fg(vp.header_path),
                ),
                Span::raw(" ".repeat(pad_len)),
                Span::styled(right_info, Style::default().fg(vp.muted)),
            ]);
            f.render_widget(
                Paragraph::new(header_line).style(content_style),
                header_rect,
            );

            let bar = if is_markdown_path(&v.file_path) && v.view_mode == ViewerMode::Text {
                Line::from(vec![
                    Span::styled(" Esc ", content_style.fg(vp.muted)),
                    Span::raw("close  "),
                    Span::styled(" T ", content_style.fg(vp.muted)),
                    Span::raw("markdown  "),
                    Span::styled(" H ", content_style.fg(vp.muted)),
                    Span::raw("hex/text  "),
                    Span::styled(" ↑↓ ", content_style.fg(vp.muted)),
                    Span::raw("PgUp/PgDn scroll"),
                ])
            } else {
                Line::from(vec![
                    Span::styled(" Esc ", content_style.fg(vp.muted)),
                    Span::raw("close  "),
                    Span::styled(" H ", content_style.fg(vp.muted)),
                    Span::raw("hex/text  "),
                    Span::styled(" ↑↓ ", content_style.fg(vp.muted)),
                    Span::raw("PgUp/PgDn scroll"),
                ])
            };
            f.render_widget(
                Paragraph::new(bar).style(content_style.fg(vp.muted)),
                bottom_rect,
            );
        }
    }

    app.viewer_screen = Some(state);
}
