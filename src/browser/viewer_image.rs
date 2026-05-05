//! F3 raster image viewer: load in background, display with [`ratatui_image`] (Kitty / iTerm / Sixel / half-blocks).
//!
//! Picker env: `OXIDE_IMAGE_SKIP_CAP_QUERY=1` skips stdin/stdout capability probing; `OXIDE_IMAGE_PROTOCOL=halfblocks`
//! (or `iterm2`, `sixel`, `kitty`) forces the graphics mode after init—`halfblocks` avoids inline protocol escapes.
//!
//! On **Apple iTerm2**, capability queries can pick Kitty or Sixel even though OSC 1337 inline images
//! are the reliable path; we normalize to `Iterm2` unless `OXIDE_IMAGE_PROTOCOL` overrides.

use std::io::{self, Cursor};
use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use image::DynamicImage;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{FilterType, Resize, StatefulImage};

use crate::app::events::AppAction;
use crate::app::state::AppState;
use crate::core::file_ops::FileOperations;
use crate::core::location::PanelLocation;
use crate::core::panel_backend;
use crate::ui::dialog_layout;
use crate::ui::theme::{DialogPalette, ViewerPalette};
use crate::util;

use crate::browser::panel::{Panel, PanelOperations};

/// How to read bytes for one image tab.
#[derive(Clone)]
pub enum ImageReadSource {
    Fs {
        path: PathBuf,
    },
    Panel {
        loc: PanelLocation,
        name: String,
    },
}

/// One tab / one image source.
#[derive(Clone)]
pub struct ImageViewEntry {
    pub tab_label: String,
    /// Full path for display in errors / future UI (not shown in the image viewer chrome today).
    #[allow(dead_code)]
    pub path_banner: String,
    pub read: ImageReadSource,
}

#[derive(Debug)]
pub enum ImageLoadMsg {
    Part {
        index: usize,
        bytes: Vec<u8>,
    },
    Failed(io::Error),
}

/// Background load of one or more images; Esc cancels.
pub struct ImageLoadingState {
    pub entries: Vec<ImageViewEntry>,
    pub current: usize,
    pub buffers: Vec<Option<Vec<u8>>>,
    /// How many [`Part`] messages have been applied (files fully read into `buffers`).
    pub loaded: usize,
    pub rx: mpsc::Receiver<ImageLoadMsg>,
    pub cancel: Arc<AtomicBool>,
}

/// Ready multi-image viewer: decoded images + tab index; drawn with [`ratatui_image`] in [`draw_image_ready`].
pub struct ImageViewerState {
    pub sources: Vec<DynamicImage>,
    pub entries: Vec<ImageViewEntry>,
    pub tab_labels: Vec<String>,
    pub current: usize,
    /// Last full layout (for mouse tab hit-test).
    pub area: Rect,
    pub tabs_area: Rect,
    /// [`StatefulProtocol`] for [`sources`][`current`]; rebuilt when `protocol_stale`.
    pub image_protocol: StatefulProtocol,
    /// Set when switching tabs so the next draw rebuilds [`image_protocol`] from [`sources`].
    pub protocol_stale: bool,
}

pub fn is_raster_image_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
}

fn read_image_bytes(
    entry: &ImageReadSource,
    cancel: &AtomicBool,
) -> io::Result<Vec<u8>> {
    match entry {
        ImageReadSource::Fs { path } => util::read_path_chunked(path, cancel),
        ImageReadSource::Panel { loc, name } => match loc {
            PanelLocation::Fs(p) => {
                let path = FileOperations::join_path(p, name);
                util::read_path_chunked(&path, cancel)
            }
            _ => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "image load cancelled",
                    ));
                }
                panel_backend::read_file(loc, name)
            }
        },
    }
}

/// Build the list of raster files to show and the initial tab index (`0` when opening).
///
/// When the panel has marks, only marked raster files are included (sorted by row).
/// The viewer always starts on the **first** tab (top of the list); use keys or mouse to switch.
/// If that list is empty but the current file is a raster image, falls back to a single-file view.
pub fn collect_raster_view_entries(
    panel: &Panel,
    loc: &PanelLocation,
) -> Option<(Vec<ImageViewEntry>, usize)> {
    fn panel_raster_entry(loc: &PanelLocation, name: &str) -> ImageViewEntry {
        ImageViewEntry {
            tab_label: name.to_string(),
            path_banner: panel_backend::join_path_display(loc, name),
            read: ImageReadSource::Panel {
                loc: loc.clone(),
                name: name.to_string(),
            },
        }
    }

    let files = panel.get_files();
    let sel = panel.get_selected_index();
    let current = files.get(sel)?;
    if current.is_dir || current.is_parent_dir() || !is_raster_image_filename(&current.name) {
        return None;
    }

    let has_marks = panel.iter_marked_indices().next().is_some();
    if !has_marks {
        return Some((vec![panel_raster_entry(loc, &current.name)], 0));
    }

    let mut indices: Vec<usize> = panel
        .iter_marked_indices()
        .filter(|&i| {
            files.get(i).is_some_and(|f| {
                !f.is_dir && !f.is_parent_dir() && is_raster_image_filename(&f.name)
            })
        })
        .collect();
    indices.sort_unstable();

    if indices.is_empty() {
        return Some((vec![panel_raster_entry(loc, &current.name)], 0));
    }

    let entries: Vec<ImageViewEntry> = indices
        .iter()
        .filter_map(|&i| {
            let f = files.get(i)?;
            Some(panel_raster_entry(loc, &f.name))
        })
        .collect();

    Some((entries, 0))
}

/// Single filesystem path (e.g. Find dialog).
pub fn single_fs_raster_entry(path: PathBuf) -> Option<(Vec<ImageViewEntry>, usize)> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    if !is_raster_image_filename(name) {
        return None;
    }
    let path_banner = path.display().to_string();
    let e = ImageViewEntry {
        tab_label: name.to_string(),
        path_banner,
        read: ImageReadSource::Fs { path },
    };
    Some((vec![e], 0))
}

pub fn spawn_image_load_thread(
    entries: Vec<ImageViewEntry>,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<ImageLoadMsg>,
) {
    std::thread::spawn(move || {
        for (index, entry) in entries.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match read_image_bytes(&entry.read, cancel.as_ref()) {
                Ok(bytes) => {
                    if tx.send(ImageLoadMsg::Part { index, bytes }).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(ImageLoadMsg::Failed(e));
                    return;
                }
            }
        }
    });
}

fn decode_image(bytes: &[u8]) -> io::Result<DynamicImage> {
    let r = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    r.decode()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// True when the host is likely iTerm2 / WezTerm / etc. and supports inline image escapes (OSC 1337).
fn env_suggests_iterm2_style_inline_images() -> bool {
    if env::var("ITERM_SESSION_ID")
        .ok()
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    if env::var("TERM_PROGRAM").ok().is_some_and(|term_program| {
        term_program.contains("iTerm")
            || term_program.contains("WezTerm")
            || term_program.contains("mintty")
            || term_program.contains("vscode")
            || term_program.contains("Tabby")
            || term_program.contains("Hyper")
            || term_program.contains("rio")
            || term_program.contains("Bobcat")
            || term_program.contains("WarpTerminal")
    }) {
        return true;
    }
    env::var("LC_TERMINAL")
        .ok()
        .is_some_and(|lc_term| lc_term.contains("iTerm"))
}

/// Apple iTerm2 (including tmux panes that inherited `ITERM_SESSION_ID`).
///
/// `ratatui_image`'s stdin/stdout capability query prefers IO-detected Kitty/Sixel over the iTerm2
/// protocol; on iTerm.app that often yields a blank image area.
fn env_is_apple_iterm_host() -> bool {
    if env::var("ITERM_SESSION_ID")
        .ok()
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    env::var("TERM_PROGRAM")
        .ok()
        .is_some_and(|p| p.contains("iTerm"))
        || env::var("LC_TERMINAL")
            .ok()
            .is_some_and(|lc| lc.contains("iTerm"))
}

/// Default (w, h) cell size in **pixels** when OSC queries fail but we still use the iTerm2 inline
/// protocol. Slightly larger than legacy 10×20 helps Retina / dense fonts map bitmaps more sharply.
fn iterm_fallback_cell_pixels() -> ratatui_image::FontSize {
    (12, 24)
}

#[inline]
fn env_truthy(var: &str) -> bool {
    env::var(var).ok().is_some_and(|v| {
        matches!(
            v.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Picker without `from_query_stdio` (no raw stdin read / stdout query).
fn picker_without_stdio_query(iterm_like: bool) -> Picker {
    if iterm_like {
        #[allow(deprecated)]
        Picker::from_fontsize(iterm_fallback_cell_pixels())
    } else {
        Picker::halfblocks()
    }
}

fn image_protocol_from_env() -> Option<ProtocolType> {
    env::var("OXIDE_IMAGE_PROTOCOL").ok().and_then(|s| {
        match s.to_ascii_lowercase().as_str() {
            "halfblocks" | "blocks" => Some(ProtocolType::Halfblocks),
            "iterm2" | "iterm" => Some(ProtocolType::Iterm2),
            "sixel" => Some(ProtocolType::Sixel),
            "kitty" => Some(ProtocolType::Kitty),
            _ => None,
        }
    })
}

fn build_image_picker() -> Picker {
    let iterm_like = env_suggests_iterm2_style_inline_images();
    let skip_stdio_query = env_truthy("OXIDE_IMAGE_SKIP_CAP_QUERY");

    let mut picker = if skip_stdio_query {
        picker_without_stdio_query(iterm_like)
    } else {
        match Picker::from_query_stdio() {
            Ok(p) => p,
            Err(_) => picker_without_stdio_query(iterm_like),
        }
    };

    // Prefer OSC 1337 on Apple iTerm2: cap query often selects Kitty/Sixel, which do not draw there.
    if env_is_apple_iterm_host() && !matches!(picker.protocol_type(), ProtocolType::Iterm2) {
        picker.set_protocol_type(ProtocolType::Iterm2);
    } else if iterm_like && matches!(picker.protocol_type(), ProtocolType::Halfblocks) {
        // WezTerm, VS Code, tmux+outer-terminal hints, etc.: only bump pure fallback halfblocks.
        picker.set_protocol_type(ProtocolType::Iterm2);
    }

    if let Some(proto) = image_protocol_from_env() {
        picker.set_protocol_type(proto);
    }

    picker
}

/// Initialize [`AppState::image_picker`] once (terminal capability / font-size query).
pub fn ensure_image_picker(app: &mut AppState) {
    if app.image_picker.is_some() {
        return;
    }
    app.image_picker = Some(build_image_picker());
}

pub fn finish_image_loading(
    loading: ImageLoadingState,
    app: &mut AppState,
) -> Option<ImageViewerState> {
    let ImageLoadingState {
        entries,
        current,
        buffers,
        ..
    } = loading;

    let mut sources = Vec::with_capacity(entries.len());
    for (i, buf_opt) in buffers.into_iter().enumerate() {
        let Some(bytes) = buf_opt else {
            app.set_timed_toast_alert(
                std::time::Duration::from_secs(3),
                "Internal error: missing image buffer.",
            );
            return None;
        };
        match decode_image(&bytes) {
            Ok(img) => {
                sources.push(img);
            }
            Err(e) => {
                let name = entries
                    .get(i)
                    .map(|e| e.tab_label.as_str())
                    .unwrap_or("?");
                app.set_timed_toast_alert(
                    std::time::Duration::from_secs(5),
                    format!("Could not decode image {name}: {e}"),
                );
                return None;
            }
        }
    }

    let tab_labels: Vec<String> = entries
        .iter()
        .map(|e| truncate_tab_label(&e.tab_label))
        .collect();

    let n = sources.len();
    let current_clamped = current.min(n.saturating_sub(1));

    ensure_image_picker(app);
    let Some(picker) = app.image_picker.as_ref() else {
        return None;
    };
    let start_img = sources
        .get(current_clamped)
        .expect("current_clamped in range")
        .clone();
    let image_protocol = picker.new_resize_protocol(start_img);

    Some(ImageViewerState {
        sources,
        tab_labels,
        entries,
        current: current_clamped,
        area: Rect::default(),
        tabs_area: Rect::default(),
        image_protocol,
        protocol_stale: false,
    })
}

fn render_spinner_frame() -> &'static str {
    const FRAMES: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];
    let i = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_millis() / 120 % 4) as usize)
        .unwrap_or(0)
        .min(3);
    FRAMES[i]
}

fn truncate_tab_label(s: &str) -> String {
    const MAX: usize = 24;
    let n = s.chars().count();
    if n <= MAX {
        return s.to_string();
    }
    s.chars()
        .take(MAX.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

fn truncate_display_width(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    let n = s.chars().count();
    if n <= max_cols {
        return s.to_string();
    }
    let take = max_cols.saturating_sub(1);
    s.chars().take(take).chain(std::iter::once('…')).collect()
}

/// Truncate to `cols`, then pad with spaces to exactly `cols` character cells.
///
/// Ratatui only paints cells covered by the `Paragraph`; shorter strings leave the rest of the
/// row unchanged, so colors from the file list (or other UI) can “bleed” as stray glyphs.
fn row_cells_fixed(s: &str, cols: usize) -> String {
    if cols == 0 {
        return String::new();
    }
    let mut t = truncate_display_width(s, cols);
    let mut n = t.chars().count();
    while n < cols {
        t.push(' ');
        n += 1;
    }
    t
}

/// Strip attributes that can survive across ratatui cells from the previous UI (e.g. panel list).
#[inline]
fn status_line_plain_style(bg: ratatui::style::Color, fg: ratatui::style::Color) -> Style {
    Style::default()
        .bg(bg)
        .fg(fg)
        .remove_modifier(
            Modifier::BOLD
                | Modifier::DIM
                | Modifier::ITALIC
                | Modifier::UNDERLINED
                | Modifier::SLOW_BLINK
                | Modifier::RAPID_BLINK
                | Modifier::REVERSED
                | Modifier::HIDDEN
                | Modifier::CROSSED_OUT,
        )
}

/// Full-width status row: erase every cell (spaces, fg = bg), then draw padded `text` with `fg` on `bg`.
fn paint_full_width_status_line(
    f: &mut Frame,
    rect: Rect,
    text: &str,
    bg: ratatui::style::Color,
    fg: ratatui::style::Color,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let w = rect.width as usize;
    let erase = status_line_plain_style(bg, bg);
    f.render_widget(
        Paragraph::new(row_cells_fixed("", w)).style(erase),
        rect,
    );
    f.render_widget(
        Paragraph::new(row_cells_fixed(text, w)).style(status_line_plain_style(bg, fg)),
        rect,
    );
}

#[inline]
fn image_resize_fit() -> Resize {
    Resize::Fit(Some(FilterType::Triangle))
}

/// Hit-test tab index for the **vertical** tab column on the left (one tab per row).
///
/// Must match [`draw_image_ready`]: row `y + i` is tab `i` for `i < tab_labels.len()`.
pub fn tab_index_at_column(
    tab_labels: &[String],
    tabs_area: Rect,
    col: u16,
    row: u16,
    _current: usize,
) -> Option<usize> {
    let n_tabs = tab_labels.len();
    if n_tabs <= 1 || tabs_area.width == 0 || tabs_area.height == 0 {
        return None;
    }
    if col < tabs_area.x || col >= tabs_area.x.saturating_add(tabs_area.width) {
        return None;
    }
    if row < tabs_area.y || row >= tabs_area.y.saturating_add(tabs_area.height) {
        return None;
    }
    let rel = (row - tabs_area.y) as usize;
    (rel < n_tabs).then_some(rel)
}

/// Width in terminal columns for the left tab strip (multi-file viewer).
///
/// Width does **not** depend on which tab is selected, so highlighting with `[` `]` does not
/// resize the column or shift the image area.
fn vertical_tab_column_width(tab_labels: &[String], area_width: u16) -> u16 {
    const MIN_IMG: u16 = 20;
    const MIN_TAB_W: u16 = 12;
    const MAX_TAB_W: u16 = 48;
    let max_allowed = area_width.saturating_sub(MIN_IMG).max(MIN_TAB_W);
    let mut need = MIN_TAB_W;
    for label in tab_labels.iter() {
        let w = label.chars().count() as u16;
        // Reserve bracket columns so any row can become active without changing strip width.
        let row = w.saturating_add(2);
        need = need.max(row.saturating_add(1));
    }
    need.clamp(MIN_TAB_W, MAX_TAB_W).min(max_allowed)
}

fn vertical_tab_lines(
    tab_labels: &[String],
    current: usize,
    vp: ViewerPalette,
    col_width: u16,
) -> Vec<Line<'static>> {
    let cw = col_width as usize;
    if cw == 0 {
        return Vec::new();
    }
    let normal = Style::default().fg(vp.text);
    let active = normal.add_modifier(Modifier::BOLD | Modifier::REVERSED);
    let mut out = Vec::with_capacity(tab_labels.len());
    for (i, label) in tab_labels.iter().enumerate() {
        let line = if i == current {
            let inner = cw.saturating_sub(2).max(1);
            let s = row_cells_fixed(label.as_str(), inner);
            Line::from(vec![
                Span::styled("[", active),
                Span::styled(s, active),
                Span::styled("]", active),
            ])
        } else {
            Line::from(Span::styled(row_cells_fixed(label.as_str(), cw), normal))
        };
        out.push(line);
    }
    out
}

/// Full-screen F3 image view: single = image with title row above footer; multi = vertical tabs left, image right.
pub fn draw_image_ready(
    f: &mut Frame,
    img: &mut ImageViewerState,
    app: &mut AppState,
    vp: ViewerPalette,
) {
    let area = f.area();
    // First paint after `ImageLoading` (or any prior UI) can retain styled cells; reset buffer.
    f.render_widget(Clear, area);

    ensure_image_picker(app);
    let Some(picker) = app.image_picker.as_ref() else {
        return;
    };

    if img.protocol_stale {
        if let Some(dyn_img) = img.sources.get(img.current).cloned() {
            img.image_protocol = picker.new_resize_protocol(dyn_img);
        }
        img.protocol_stale = false;
    }

    img.area = area;

    let content_style = Style::default().bg(vp.background).fg(vp.text);
    let footer_h = 1u16;
    let chrome_h = 1u16;
    let footer_rect = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(footer_h),
        width: area.width,
        height: footer_h,
    };

    let n = img.entries.len();
    if n <= 1 {
        img.tabs_area = Rect {
            x: area.x,
            y: area.y,
            width: 0,
            height: 0,
        };
        let img_rect = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height.saturating_sub(footer_h + chrome_h),
        };
        f.render_widget(Block::default().style(content_style), img_rect);
        let image_widget = StatefulImage::default().resize(image_resize_fit());
        f.render_stateful_widget(image_widget, img_rect, &mut img.image_protocol);

        let title_rect = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(footer_h + chrome_h),
            width: area.width,
            height: chrome_h,
        };
        let title_text = img
            .entries
            .get(img.current)
            .map(|e| e.tab_label.as_str())
            .unwrap_or("");
        paint_full_width_status_line(f, title_rect, title_text, vp.background, vp.header_path);
    } else {
        let content_h = area.height.saturating_sub(footer_h);
        let tabs_w = vertical_tab_column_width(&img.tab_labels, area.width);
        let tabs_rect = Rect {
            x: area.x,
            y: area.y,
            width: tabs_w,
            height: content_h,
        };
        let img_rect = Rect {
            x: area.x.saturating_add(tabs_w),
            y: area.y,
            width: area.width.saturating_sub(tabs_w),
            height: content_h,
        };
        img.tabs_area = tabs_rect;

        f.render_widget(Block::default().style(content_style), tabs_rect);
        let tab_text = vertical_tab_lines(&img.tab_labels, img.current, vp, tabs_w);
        f.render_widget(
            Paragraph::new(Text::from(tab_text)).style(content_style),
            tabs_rect,
        );

        f.render_widget(Block::default().style(content_style), img_rect);
        let image_widget = StatefulImage::default().resize(image_resize_fit());
        f.render_stateful_widget(image_widget, img_rect, &mut img.image_protocol);
    }

    if let Some(res) = img.image_protocol.last_encoding_result() {
        if let Err(e) = res {
            app.set_timed_toast_alert(
                Duration::from_secs(4),
                format!("Image display: {e}"),
            );
        }
    }

    // ASCII only: Unicode arrows are often double-width in terminals while ratatui counts one
    // column per char, so the row can spill and leave stale styled cells from the panel list.
    let hint = if img.entries.len() > 1 {
        "Esc: close  Up/Down or Left/Right: tab  Click tab list on the left"
    } else {
        "Esc: close"
    };
    paint_full_width_status_line(f, footer_rect, hint, vp.background, vp.muted);
}

pub fn draw_image_loading(
    f: &mut Frame,
    _ld: &ImageLoadingState,
    vp: ViewerPalette,
    dialog: DialogPalette,
) {
    let area = f.area();
    let content_style = Style::default().bg(vp.background).fg(vp.text);

    // First frame after opening from panels: ratatui can merge with old cells; wipe then paint.
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(content_style), area);
    f.render_widget(Block::default().style(dialog.dim_layer_style()), area);

    let dlg_h = 5u16.min(area.height.saturating_sub(4).max(3));
    let dlg_w = 36u16.min(area.width.saturating_sub(8)).max(28);
    let dlg = dialog_layout::centered_dialog_rect(area, dlg_w, dlg_h);
    let dlg_block = Block::default()
        .title(" Loading ")
        .borders(Borders::ALL)
        .style(dialog.border_block_style());
    f.render_widget(dlg_block, dlg);

    let inner = dialog_layout::dialog_content_rect(dlg, dialog_layout::DEFAULT_PAD_H);
    let spin = render_spinner_frame();
    let line = Line::from(vec![
        Span::styled(spin, dialog.fill_style()),
        Span::styled("  Please wait…", dialog.fill_style()),
    ]);
    f.render_widget(
        Paragraph::new(line)
            .style(dialog.fill_style())
            .alignment(Alignment::Center),
        inner,
    );

    let footer_rect = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    paint_full_width_status_line(f, footer_rect, " Esc: cancel ", vp.background, vp.muted);
}

pub fn handle_image_loading_key(key: KeyCode) -> Option<AppAction> {
    (key == KeyCode::Esc || key == KeyCode::Char('\x1b')).then_some(AppAction::ViewerClose)
}

pub fn handle_image_ready_key(
    img: &mut ImageViewerState,
    key: KeyEvent,
) -> Option<AppAction> {
    if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
        return Some(AppAction::ViewerClose);
    }
    if img.entries.len() > 1 {
        let max_i = img.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Left | KeyCode::Up => {
                let old = img.current;
                img.current = img.current.saturating_sub(1);
                if old != img.current {
                    img.protocol_stale = true;
                }
            }
            KeyCode::Right | KeyCode::Down => {
                let old = img.current;
                img.current = (img.current + 1).min(max_i);
                if old != img.current {
                    img.protocol_stale = true;
                }
            }
            _ => {}
        }
    }
    Some(AppAction::Continue)
}

pub fn handle_image_loading_mouse(mouse_event: &MouseEvent) -> bool {
    matches!(
        mouse_event.kind,
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown | MouseEventKind::Moved
    ) || matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Left))
}

pub fn handle_image_ready_mouse(
    img: &mut ImageViewerState,
    mouse_event: MouseEvent,
) -> bool {
    match mouse_event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let col = mouse_event.column;
            let row = mouse_event.row;
            if let Some(idx) =
                tab_index_at_column(&img.tab_labels, img.tabs_area, col, row, img.current)
            {
                if idx < img.entries.len() && idx != img.current {
                    img.current = idx;
                    img.protocol_stale = true;
                }
            }
            true
        }
        _ => true,
    }
}
