//! F3 markdown viewer: rendered preview via [`ratatui_markdown`] with link navigation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_markdown::highlight::{HighlightHooks, TreeSitterHighlighter};
use ratatui_markdown::markdown::{MarkdownBlock, MarkdownRenderer};
use ratatui_markdown::theme::{Generation, RichTextTheme};

use crate::app::events::AppAction;
use crate::app::state::{AppState, MarkdownNavEntry};
use crate::browser::viewer::open_viewer_path;
use crate::ui::theme::ViewerPalette;

use super::viewer_image::ensure_image_picker;
use super::viewer_markdown_images::{
    base_dir_for_markdown, draw_markdown_images, EmbeddedMarkdownImage, MarkdownImageScrollGate,
    OxideMarkdownImageResolver,
};

use super::viewer::{ViewerMode, ViewerScreenState, ViewerState};
use super::viewer_markdown_links::{
    extract_links, local_markdown_exists, resolve_markdown_link, slugify_heading, ExtractedLink,
    ResolvedLink,
};

const VIEWER_MOUSE_SCROLL_LINES: u16 = 3;

/// True when the basename looks like a markdown document.
pub fn is_markdown_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx")
}

pub fn is_markdown_path(path: &str) -> bool {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(is_markdown_filename)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkHitbox {
    pub doc_line: usize,
    pub col_start: usize,
    pub col_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownLink {
    pub url: String,
    pub label: String,
    pub hitboxes: Vec<LinkHitbox>,
}

pub struct MarkdownViewerState {
    pub file_path: String,
    pub content: String,
    pub lines: Vec<Line<'static>>,
    pub images: Vec<EmbeddedMarkdownImage>,
    pub links: Vec<MarkdownLink>,
    pub active_link: Option<usize>,
    pub scroll: u16,
    pub area: Rect,
    pub content_rect: Rect,
    pub render_width: u16,
    pub pending_fragment: Option<String>,
    image_scroll_gate: MarkdownImageScrollGate,
}

pub fn build_markdown_viewer_state(
    file_path: String,
    content: String,
) -> MarkdownViewerState {
    MarkdownViewerState {
        file_path,
        content,
        lines: Vec::new(),
        images: Vec::new(),
        links: Vec::new(),
        active_link: None,
        scroll: 0,
        area: Rect::default(),
        content_rect: Rect::default(),
        render_width: 0,
        pending_fragment: None,
        image_scroll_gate: MarkdownImageScrollGate::new(),
    }
}

pub fn clear_markdown_nav(app: &mut AppState) {
    app.markdown_viewer_nav_stack.clear();
    app.markdown_viewer_restore_scroll = None;
}

fn to_md_color(c: Color) -> ratatui029::style::Color {
    match c {
        Color::Reset => ratatui029::style::Color::Reset,
        Color::Black => ratatui029::style::Color::Black,
        Color::Red => ratatui029::style::Color::Red,
        Color::Green => ratatui029::style::Color::Green,
        Color::Yellow => ratatui029::style::Color::Yellow,
        Color::Blue => ratatui029::style::Color::Blue,
        Color::Magenta => ratatui029::style::Color::Magenta,
        Color::Cyan => ratatui029::style::Color::Cyan,
        Color::Gray => ratatui029::style::Color::Gray,
        Color::DarkGray => ratatui029::style::Color::DarkGray,
        Color::LightRed => ratatui029::style::Color::LightRed,
        Color::LightGreen => ratatui029::style::Color::LightGreen,
        Color::LightYellow => ratatui029::style::Color::LightYellow,
        Color::LightBlue => ratatui029::style::Color::LightBlue,
        Color::LightMagenta => ratatui029::style::Color::LightMagenta,
        Color::LightCyan => ratatui029::style::Color::LightCyan,
        Color::White => ratatui029::style::Color::White,
        Color::Rgb(r, g, b) => ratatui029::style::Color::Rgb(r, g, b),
        Color::Indexed(i) => ratatui029::style::Color::Indexed(i),
    }
}

fn from_md_color(c: ratatui029::style::Color) -> Color {
    match c {
        ratatui029::style::Color::Reset => Color::Reset,
        ratatui029::style::Color::Black => Color::Black,
        ratatui029::style::Color::Red => Color::Red,
        ratatui029::style::Color::Green => Color::Green,
        ratatui029::style::Color::Yellow => Color::Yellow,
        ratatui029::style::Color::Blue => Color::Blue,
        ratatui029::style::Color::Magenta => Color::Magenta,
        ratatui029::style::Color::Cyan => Color::Cyan,
        ratatui029::style::Color::Gray => Color::Gray,
        ratatui029::style::Color::DarkGray => Color::DarkGray,
        ratatui029::style::Color::LightRed => Color::LightRed,
        ratatui029::style::Color::LightGreen => Color::LightGreen,
        ratatui029::style::Color::LightYellow => Color::LightYellow,
        ratatui029::style::Color::LightBlue => Color::LightBlue,
        ratatui029::style::Color::LightMagenta => Color::LightMagenta,
        ratatui029::style::Color::LightCyan => Color::LightCyan,
        ratatui029::style::Color::White => Color::White,
        ratatui029::style::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
        ratatui029::style::Color::Indexed(i) => Color::Indexed(i),
    }
}

fn from_md_style(s: ratatui029::style::Style) -> Style {
    let mut out = Style::default();
    if s.add_modifier.contains(ratatui029::style::Modifier::BOLD) {
        out = out.bold();
    }
    if s.add_modifier.contains(ratatui029::style::Modifier::DIM) {
        out = out.dim();
    }
    if s.add_modifier.contains(ratatui029::style::Modifier::ITALIC) {
        out = out.italic();
    }
    if s.add_modifier
        .contains(ratatui029::style::Modifier::UNDERLINED)
    {
        out = out.underlined();
    }
    if s.add_modifier
        .contains(ratatui029::style::Modifier::SLOW_BLINK)
    {
        out = out.slow_blink();
    }
    if s.add_modifier
        .contains(ratatui029::style::Modifier::RAPID_BLINK)
    {
        out = out.rapid_blink();
    }
    if s.add_modifier
        .contains(ratatui029::style::Modifier::REVERSED)
    {
        out = out.reversed();
    }
    if s.add_modifier.contains(ratatui029::style::Modifier::HIDDEN) {
        out = out.hidden();
    }
    if s.add_modifier
        .contains(ratatui029::style::Modifier::CROSSED_OUT)
    {
        out = out.crossed_out();
    }
    if let Some(fg) = s.fg {
        out = out.fg(from_md_color(fg));
    }
    if let Some(bg) = s.bg {
        out = out.bg(from_md_color(bg));
    }
    out
}

fn from_md_span(span: ratatui029::text::Span<'static>) -> Span<'static> {
    Span::styled(span.content, from_md_style(span.style))
}

fn from_md_line(line: ratatui029::text::Line<'static>) -> Line<'static> {
    Line::from(line.spans.into_iter().map(from_md_span).collect::<Vec<_>>())
}

struct OxideMarkdownTheme(ViewerPalette);

impl RichTextTheme for OxideMarkdownTheme {
    fn generation(&self) -> Generation {
        Generation(1)
    }

    fn get_text_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.text)
    }

    fn get_muted_text_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.muted)
    }

    fn get_primary_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.header_path)
    }

    fn get_popup_selected_background(&self) -> ratatui029::style::Color {
        to_md_color(self.0.hex_cursor_bg)
    }

    fn get_border_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.muted)
    }

    fn get_focused_border_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.text)
    }

    fn get_secondary_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.header_path)
    }

    fn get_info_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.header_path)
    }

    fn get_json_key_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.header_path)
    }

    fn get_json_string_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.text)
    }

    fn get_json_number_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.text)
    }

    fn get_json_bool_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.text)
    }

    fn get_json_null_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.muted)
    }

    fn get_accent_yellow(&self) -> ratatui029::style::Color {
        to_md_color(self.0.header_path)
    }

    fn get_background_color(&self) -> ratatui029::style::Color {
        to_md_color(self.0.background)
    }
}

fn line_text(line: &Line<'static>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn display_width_str(s: &str) -> usize {
    Line::from(Span::raw(s.to_string())).width()
}

fn cols_for_byte_range(
    line: &Line<'static>,
    start_byte: usize,
    end_byte: usize,
) -> (usize, usize) {
    let flat = line_text(line);
    let before = &flat[..start_byte.min(flat.len())];
    let mid = &flat[start_byte.min(flat.len())..end_byte.min(flat.len())];
    (
        display_width_str(before),
        display_width_str(before) + display_width_str(mid),
    )
}

/// True when `c` may continue an alphanumeric token (ASCII labels/URLs).
fn is_label_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// Like [`str::find`], but skips matches where `needle` is only a prefix/suffix of a longer token.
fn find_whole_needle(
    haystack: &str,
    needle: &str,
    from: usize,
) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut search_from = from;
    while search_from <= haystack.len().saturating_sub(needle.len()) {
        let rel = haystack[search_from..].find(needle)?;
        let abs = search_from + rel;
        let before_ok = abs == 0
            || haystack[..abs]
                .chars()
                .last()
                .is_none_or(|c| !is_label_word_char(c));
        let after_byte = abs + needle.len();
        let after_ok = after_byte >= haystack.len()
            || haystack[after_byte..]
                .chars()
                .next()
                .is_none_or(|c| !is_label_word_char(c));
        if before_ok && after_ok {
            return Some(abs);
        }
        search_from = abs + 1;
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UsedHitRange {
    doc_line: usize,
    byte_start: usize,
    byte_end: usize,
}

fn ranges_overlap(
    a_start: usize,
    a_end: usize,
    b_start: usize,
    b_end: usize,
) -> bool {
    a_start < b_end && b_start < a_end
}

fn hit_range_used(
    used: &[UsedHitRange],
    doc_line: usize,
    byte_start: usize,
    byte_end: usize,
) -> bool {
    used.iter().any(|u| {
        u.doc_line == doc_line
            && ranges_overlap(
                u.byte_start,
                u.byte_end,
                byte_start,
                byte_end,
            )
    })
}

fn assign_hitbox(
    links: &mut Vec<MarkdownLink>,
    used: &mut Vec<UsedHitRange>,
    url: &str,
    label: &str,
    line: &Line<'static>,
    doc_line: usize,
    byte_start: usize,
    byte_end: usize,
) {
    let (col_start, col_end) = cols_for_byte_range(line, byte_start, byte_end);
    links.push(MarkdownLink {
        url: url.to_string(),
        label: label.to_string(),
        hitboxes: vec![LinkHitbox {
            doc_line,
            col_start,
            col_end,
        }],
    });
    used.push(UsedHitRange {
        doc_line,
        byte_start,
        byte_end,
    });
}

fn find_label_hitbox(
    lines: &[Line<'static>],
    used: &[UsedHitRange],
    needle: &str,
) -> Option<(usize, usize, usize)> {
    for (doc_line, line) in lines.iter().enumerate() {
        let flat = line_text(line);
        let mut search_from = 0usize;
        while let Some(byte_start) = find_whole_needle(&flat, needle, search_from) {
            let byte_end = byte_start + needle.len();
            if !hit_range_used(used, doc_line, byte_start, byte_end) {
                return Some((doc_line, byte_start, byte_end));
            }
            search_from = byte_start + 1;
        }
    }
    None
}

fn build_link_hitboxes(
    lines: &[Line<'static>],
    extracted: &[ExtractedLink],
) -> Vec<MarkdownLink> {
    let mut links = Vec::with_capacity(extracted.len());
    let mut used = Vec::new();

    for ex in extracted {
        let needle = if ex.label.is_empty() {
            ex.url.as_str()
        } else {
            ex.label.as_str()
        };
        if needle.is_empty() {
            continue;
        }
        let label = if ex.label.is_empty() {
            ex.url.as_str()
        } else {
            ex.label.as_str()
        };

        if let Some((doc_line, byte_start, byte_end)) = find_label_hitbox(lines, &used, needle) {
            assign_hitbox(
                &mut links,
                &mut used,
                &ex.url,
                label,
                &lines[doc_line],
                doc_line,
                byte_start,
                byte_end,
            );
            continue;
        }

        // Fallback: match URL text when label differs from rendered output.
        if ex.url != needle {
            if let Some((doc_line, byte_start, byte_end)) = find_label_hitbox(lines, &used, &ex.url)
            {
                assign_hitbox(
                    &mut links,
                    &mut used,
                    &ex.url,
                    label,
                    &lines[doc_line],
                    doc_line,
                    byte_start,
                    byte_end,
                );
            }
        }
    }

    links
}

fn find_link_at(
    links: &[MarkdownLink],
    doc_line: usize,
    col: usize,
) -> Option<usize> {
    links
        .iter()
        .enumerate()
        .find(|(_, link)| {
            link.hitboxes
                .iter()
                .any(|h| h.doc_line == doc_line && col >= h.col_start && col < h.col_end)
        })
        .map(|(i, _)| i)
}

fn heading_block_text(block: &MarkdownBlock) -> Option<&str> {
    match block {
        MarkdownBlock::Heading1(t) | MarkdownBlock::Heading2(t) | MarkdownBlock::Heading3(t) => {
            Some(t.as_str())
        }
        _ => None,
    }
}

fn fragment_to_doc_scroll(
    markdown: &str,
    fragment: &str,
    vp: ViewerPalette,
    width: u16,
) -> Option<u16> {
    if width == 0 {
        return None;
    }
    let want = slugify_heading(fragment);
    if want.is_empty() {
        return None;
    }
    let theme = OxideMarkdownTheme(vp);
    let renderer = markdown_renderer(width as usize, vp);
    let blocks = renderer.parse(markdown);
    let mut doc_line = 0usize;
    for block in &blocks {
        if let Some(text) = heading_block_text(block) {
            if slugify_heading(text) == want {
                return Some(doc_line.min(u16::MAX as usize) as u16);
            }
        }
        let rendered = renderer.render(std::slice::from_ref(block), &theme);
        doc_line += rendered.len();
    }
    None
}

fn visible_lines(area: Rect) -> u16 {
    area.height.saturating_sub(2).max(1)
}

/// Markdown renderer with tree-sitter code-block highlighting (shared layout for scroll anchors).
fn markdown_renderer(
    width: usize,
    vp: ViewerPalette,
) -> MarkdownRenderer {
    let highlighter = Arc::new(TreeSitterHighlighter::new());
    let hooks = HighlightHooks::new(highlighter, width).with_border_color(to_md_color(vp.muted));
    MarkdownRenderer::new(width).with_render_hooks(Box::new(hooks))
}

fn ensure_rendered(
    md: &mut MarkdownViewerState,
    vp: ViewerPalette,
    picker: &Picker,
) {
    let width = md.area.width;
    if width == 0 {
        return;
    }
    if md.render_width != width || md.lines.is_empty() {
        md.render_width = width;
        let theme = OxideMarkdownTheme(vp);
        let renderer = markdown_renderer(width as usize, vp);
        let base_dir = base_dir_for_markdown(&md.file_path);
        let mut resolver = OxideMarkdownImageResolver::new(base_dir, picker);
        let (blocks, resolved) = renderer.parse_with_images(&md.content, &mut resolver);
        let max_h = visible_lines(md.area).max(1).saturating_mul(50);
        let output = renderer.render_full(
            &blocks,
            &theme,
            &resolved,
            &mut resolver,
            width,
            max_h,
        );
        md.lines = output.lines.into_iter().map(from_md_line).collect();
        md.images = output
            .images
            .into_iter()
            .map(|p| EmbeddedMarkdownImage::from_placement(p, picker))
            .collect();
        let extracted = extract_links(&md.content);
        md.links = build_link_hitboxes(&md.lines, &extracted);
        if md.active_link.is_some_and(|i| i >= md.links.len()) {
            md.active_link = None;
        }
        if let Some(ref frag) = md.pending_fragment.clone() {
            if let Some(s) = fragment_to_doc_scroll(&md.content, frag, vp, md.render_width) {
                md.scroll = s;
            }
            md.pending_fragment = None;
        }
        md.clamp_scroll();
    }
}

impl MarkdownViewerState {
    fn doc_h(&self) -> u16 {
        self.lines.len() as u16
    }

    fn content_h(&self) -> u16 {
        visible_lines(self.area)
    }

    fn clamp_scroll(&mut self) {
        let max = self.doc_h().saturating_sub(self.content_h());
        if self.scroll > max {
            self.scroll = max;
        }
    }

    fn apply_scroll(
        &mut self,
        new_scroll: u16,
    ) {
        if self.scroll != new_scroll {
            self.scroll = new_scroll;
            self.image_scroll_gate.note_scroll();
        }
    }

    fn scroll_up(
        &mut self,
        n: u16,
    ) {
        self.apply_scroll(self.scroll.saturating_sub(n));
    }

    fn scroll_down(
        &mut self,
        n: u16,
    ) {
        let next = self.scroll.saturating_add(n);
        self.apply_scroll(next);
        self.clamp_scroll();
    }

    fn page_up(&mut self) {
        let step = self.content_h().max(1);
        self.apply_scroll(self.scroll.saturating_sub(step));
    }

    fn page_down(&mut self) {
        let step = self.content_h().max(1);
        let next = self.scroll.saturating_add(step);
        self.apply_scroll(next);
        self.clamp_scroll();
    }

    /// True when `doc_line` is within the current viewport.
    fn is_line_visible(
        &self,
        doc_line: usize,
    ) -> bool {
        let top = self.scroll as usize;
        let vh = self.content_h() as usize;
        if vh == 0 {
            return true;
        }
        doc_line >= top && doc_line < top + vh
    }

    /// Minimal scroll so `doc_line` is visible; no-op when already on screen.
    fn scroll_to_show_line(
        &mut self,
        doc_line: usize,
    ) {
        if self.is_line_visible(doc_line) {
            return;
        }
        let vh = self.content_h().max(1) as usize;
        let top = self.scroll as usize;
        let new_scroll = if doc_line < top {
            doc_line as u16
        } else {
            doc_line.saturating_sub(vh - 1) as u16
        };
        self.apply_scroll(new_scroll);
        self.clamp_scroll();
    }

    fn cycle_link(
        &mut self,
        forward: bool,
    ) {
        if self.links.is_empty() {
            self.active_link = None;
            return;
        }
        let n = self.links.len();
        let prev = self.active_link;
        let next = match prev {
            None => {
                if forward {
                    0
                } else {
                    n - 1
                }
            }
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
        };
        self.active_link = Some(next);
        if prev == Some(next) {
            return;
        }
        if let Some(h) = self.links[next].hitboxes.first() {
            self.scroll_to_show_line(h.doc_line);
        }
    }
}

fn apply_link_highlight(
    line: &Line<'static>,
    doc_line: usize,
    active: Option<usize>,
    links: &[MarkdownLink],
    highlight: Style,
) -> Line<'static> {
    let Some(active) = active else {
        return line.clone();
    };
    let link = match links.get(active) {
        Some(l) => l,
        None => return line.clone(),
    };
    let mut col = 0usize;
    let mut spans: Vec<Span<'static>> = Vec::new();
    for span in &line.spans {
        let w = span.width();
        let mut style = span.style;
        for h in &link.hitboxes {
            if h.doc_line == doc_line {
                let end = col + w;
                if col < h.col_end && end > h.col_start {
                    style = highlight;
                    break;
                }
            }
        }
        spans.push(Span::styled(span.content.clone(), style));
        col += w;
    }
    Line::from(spans)
}

fn padded_visible_lines(
    lines: &[Line<'static>],
    scroll: usize,
    visible: usize,
    inner_w: usize,
    active_link: Option<usize>,
    links: &[MarkdownLink],
    highlight: Style,
) -> Vec<Line<'static>> {
    let blank = Line::from(Span::raw(" ".repeat(inner_w)));
    let mut padded: Vec<Line<'static>> = Vec::with_capacity(visible);

    for (vis_idx, line) in lines.iter().skip(scroll).take(visible).enumerate() {
        let doc_line = scroll + vis_idx;
        let line = apply_link_highlight(
            line,
            doc_line,
            active_link,
            links,
            highlight,
        );
        let spans = line.spans.clone();
        let used: usize = spans.iter().map(|s| s.width()).sum();
        if used < inner_w {
            let mut s = spans;
            s.push(Span::raw(" ".repeat(inner_w - used)));
            padded.push(Line::from(s));
        } else if used > inner_w {
            let mut taken = 0usize;
            let mut short: Vec<Span<'static>> = Vec::new();
            for sp in spans {
                let sp_w = sp.width();
                if taken + sp_w > inner_w {
                    let keep = inner_w - taken;
                    let chop: String = sp.content.chars().take(keep).collect();
                    short.push(Span::styled(chop, sp.style));
                    break;
                }
                taken += sp_w;
                short.push(sp);
            }
            while taken < inner_w {
                short.push(Span::raw(" "));
                taken += 1;
            }
            padded.push(Line::from(short));
        } else {
            padded.push(Line::from(spans));
        }
    }
    while padded.len() < visible {
        padded.push(blank.clone());
    }
    padded
}

pub fn draw_markdown(
    f: &mut Frame,
    md: &mut MarkdownViewerState,
    app: &mut AppState,
    vp: ViewerPalette,
) {
    ensure_image_picker(app);
    let picker = app
        .image_picker
        .as_ref()
        .expect("ensure_image_picker just initialized picker");

    md.area = f.area();
    ensure_rendered(md, vp, picker);

    let area = md.area;
    let content_style = Style::default().bg(vp.background).fg(vp.text);
    let link_highlight = Style::default()
        .bg(vp.hex_cursor_bg)
        .fg(vp.hex_cursor_fg)
        .add_modifier(Modifier::UNDERLINED);
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
    md.content_rect = content_rect;
    let bottom_rect = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(bottom_height),
        width: area.width,
        height: bottom_height,
    };

    f.render_widget(
        Block::default().style(content_style),
        content_rect,
    );

    let inner_w = content_rect.width as usize;
    let scroll = md.scroll as usize;
    let visible = padded_visible_lines(
        &md.lines,
        scroll,
        content_height_usize,
        inner_w,
        md.active_link,
        &md.links,
        link_highlight,
    );
    f.render_widget(
        Paragraph::new(Text::from(visible)).style(content_style),
        content_rect,
    );

    if md.image_scroll_gate.should_draw(!md.images.is_empty()) {
        draw_markdown_images(
            f,
            &mut md.images,
            picker,
            content_rect,
            md.scroll,
        );
    }

    let total = md.doc_h() as usize;
    let link_count = md.links.len();
    let mode_label = "MARKDOWN";
    let path_span = md.file_path.as_str();
    let right_info = format!("{mode_label} | {total} lines | {link_count} links");
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

    let bar = Line::from(vec![
        Span::styled(" Esc ", content_style.fg(vp.muted)),
        Span::raw("close  "),
        Span::styled(" Tab ", content_style.fg(vp.muted)),
        Span::raw("link  "),
        Span::styled(" Enter ", content_style.fg(vp.muted)),
        Span::raw("follow  "),
        Span::styled(" ⌫ ", content_style.fg(vp.muted)),
        Span::raw("back  "),
        Span::styled(" T ", content_style.fg(vp.muted)),
        Span::raw("raw"),
    ]);
    f.render_widget(
        Paragraph::new(bar).style(content_style.fg(vp.muted)),
        bottom_rect,
    );
}

struct MarkdownFollowSnapshot {
    file_path: String,
    content: String,
    scroll: u16,
    links: Vec<MarkdownLink>,
    render_width: u16,
    area_width: u16,
}

fn push_nav_entry(
    app: &mut AppState,
    md: &MarkdownFollowSnapshot,
) {
    app.markdown_viewer_nav_stack.push(MarkdownNavEntry {
        path: PathBuf::from(&md.file_path),
        scroll: md.scroll,
    });
}

fn follow_markdown_link(
    app: &mut AppState,
    md: &MarkdownFollowSnapshot,
    link_index: usize,
) -> bool {
    let Some(link) = md.links.get(link_index) else {
        return false;
    };
    let base = Path::new(&md.file_path);
    match resolve_markdown_link(base, &link.url) {
        ResolvedLink::External { .. } => {
            app.set_timed_toast_alert(
                std::time::Duration::from_secs(3),
                "External links are not opened from the viewer.",
            );
            false
        }
        ResolvedLink::SameFile { fragment } => {
            if fragment.is_empty() {
                return false;
            }
            if let Some(scroll) = fragment_to_doc_scroll(
                &md.content,
                &fragment,
                app.ui_palette.viewer,
                md.render_width.max(md.area_width),
            ) {
                if let Some(ViewerState::MarkdownReady(cur)) = app.viewer_screen.as_mut() {
                    cur.scroll = scroll;
                    cur.clamp_scroll();
                }
            } else {
                app.set_timed_toast_alert(
                    std::time::Duration::from_secs(3),
                    "Heading anchor not found.",
                );
            }
            true
        }
        ResolvedLink::LocalFile { path, fragment } => {
            if !local_markdown_exists(&path) {
                app.set_timed_toast_alert(
                    std::time::Duration::from_secs(4),
                    format!(
                        "Markdown file not found: {}",
                        path.display()
                    ),
                );
                return false;
            }
            push_nav_entry(app, md);
            app.markdown_viewer_follow_link = true;
            app.markdown_viewer_pending_fragment = fragment;
            open_viewer_path(app, path, None);
            true
        }
    }
}

pub fn markdown_nav_back(app: &mut AppState) -> bool {
    let Some(entry) = app.markdown_viewer_nav_stack.pop() else {
        return false;
    };
    app.markdown_viewer_restore_scroll = Some(entry.scroll);
    app.markdown_viewer_follow_link = true;
    open_viewer_path(app, entry.path, None);
    true
}

pub fn handle_markdown_key(
    app: &mut AppState,
    key: KeyEvent,
) -> Option<AppAction> {
    if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
        return Some(AppAction::ViewerClose);
    }
    if key.code == KeyCode::Char('t') || key.code == KeyCode::Char('T') {
        let taken = app.viewer_screen.take();
        if let Some(ViewerState::MarkdownReady(md)) = taken {
            app.viewer_screen = Some(ViewerState::Ready(
                markdown_to_text_viewer(md),
            ));
        }
        return Some(AppAction::Continue);
    }
    if key.code == KeyCode::Backspace || key.code == KeyCode::Char('\u{7f}') {
        if markdown_nav_back(app) {
            return Some(AppAction::Continue);
        }
    }
    let code = match key.code {
        KeyCode::Char('\t') => KeyCode::Tab,
        other => other,
    };
    let Some(ViewerState::MarkdownReady(md)) = app.viewer_screen.as_mut() else {
        return Some(AppAction::Continue);
    };
    match code {
        KeyCode::Tab => {
            if md.links.is_empty() {
                app.set_timed_toast_alert(
                    std::time::Duration::from_secs(2),
                    "No links found in this document.",
                );
            } else {
                let forward = !key.modifiers.contains(KeyModifiers::SHIFT);
                md.cycle_link(forward);
            }
        }
        KeyCode::BackTab => {
            if !md.links.is_empty() {
                md.cycle_link(false);
            }
        }
        KeyCode::Enter => {
            let link_idx = md.active_link;
            let snap = MarkdownFollowSnapshot {
                file_path: md.file_path.clone(),
                content: md.content.clone(),
                scroll: md.scroll,
                links: md.links.clone(),
                render_width: md.render_width,
                area_width: md.area.width,
            };
            if let Some(i) = link_idx {
                follow_markdown_link(app, &snap, i);
            } else if !md.links.is_empty() {
                md.active_link = Some(0);
            }
        }
        KeyCode::Up => md.scroll_up(1),
        KeyCode::Down => md.scroll_down(1),
        KeyCode::PageUp => md.page_up(),
        KeyCode::PageDown => md.page_down(),
        KeyCode::Home => md.apply_scroll(0),
        KeyCode::End => md.apply_scroll(md.doc_h().saturating_sub(md.content_h())),
        _ => {}
    }
    Some(AppAction::Continue)
}

pub fn handle_markdown_mouse(
    app: &mut AppState,
    mouse_event: MouseEvent,
) -> bool {
    let Some(ViewerState::MarkdownReady(md)) = app.viewer_screen.as_mut() else {
        return true;
    };
    match mouse_event.kind {
        MouseEventKind::ScrollUp => md.scroll_up(VIEWER_MOUSE_SCROLL_LINES),
        MouseEventKind::ScrollDown => md.scroll_down(VIEWER_MOUSE_SCROLL_LINES),
        MouseEventKind::Down(MouseButton::Left) => {
            let (x, y) = (mouse_event.column, mouse_event.row);
            let r = md.content_rect;
            if r.width == 0 || r.height == 0 {
                return true;
            }
            if x < r.x || x >= r.x + r.width || y < r.y || y >= r.y + r.height {
                return true;
            }
            let doc_line = md.scroll as usize + (y - r.y) as usize;
            let col = (x - r.x) as usize;
            if let Some(i) = find_link_at(&md.links, doc_line, col) {
                md.active_link = Some(i);
                let snap = MarkdownFollowSnapshot {
                    file_path: md.file_path.clone(),
                    content: md.content.clone(),
                    scroll: md.scroll,
                    links: md.links.clone(),
                    render_width: md.render_width,
                    area_width: md.area.width,
                };
                follow_markdown_link(app, &snap, i);
            }
        }
        _ => {}
    }
    true
}

pub fn on_markdown_ready(
    md: &mut MarkdownViewerState,
    app: &mut AppState,
) {
    if let Some(scroll) = app.markdown_viewer_restore_scroll.take() {
        md.scroll = scroll;
    }
    if let Some(frag) = app.markdown_viewer_pending_fragment.take() {
        md.pending_fragment = Some(frag);
    }
}

pub fn markdown_to_text_viewer(md: MarkdownViewerState) -> ViewerScreenState {
    ViewerScreenState {
        file_path: md.file_path,
        content: md.content.into_bytes(),
        view_mode: ViewerMode::Text,
        scroll: 0,
        hex_cursor: 0,
        area: md.area,
        text_line_starts: None,
        text_display_cumulative: None,
        text_cache_width: 0,
    }
}

pub fn text_viewer_to_markdown(v: ViewerScreenState) -> Option<MarkdownViewerState> {
    if !is_markdown_path(&v.file_path) {
        return None;
    }
    let content = String::from_utf8(v.content).ok()?;
    Some(build_markdown_viewer_state(
        v.file_path,
        content,
    ))
}

pub fn try_open_markdown_from_bytes(
    file_path: &str,
    content: &[u8],
) -> Option<MarkdownViewerState> {
    if !is_markdown_path(file_path) {
        return None;
    }
    let text = std::str::from_utf8(content).ok()?.to_owned();
    Some(build_markdown_viewer_state(
        file_path.to_string(),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_hitboxes_single_line_link() {
        let extracted = vec![ExtractedLink {
            url: "./x.md".into(),
            label: "click".into(),
        }];
        let lines = vec![Line::from(vec![Span::styled(
            "before click after",
            Style::default(),
        )])];
        let links = build_link_hitboxes(&lines, &extracted);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].hitboxes[0].doc_line, 0);
        assert_eq!(links[0].hitboxes[0].col_start, 7);
        assert_eq!(links[0].hitboxes[0].col_end, 12);
    }

    #[test]
    fn build_hitboxes_bare_url() {
        let extracted = vec![ExtractedLink {
            url: "https://example.com/doc.md".into(),
            label: "https://example.com/doc.md".into(),
        }];
        let lines = vec![Line::from("visit https://example.com/doc.md today")];
        let links = build_link_hitboxes(&lines, &extracted);
        assert_eq!(links.len(), 1);
        assert!(links[0].hitboxes[0].col_start > 0);
    }

    #[test]
    fn build_hitboxes_chapter_1_not_prefix_of_chapter_11() {
        let line = Line::from("See Chapter 11 and Chapter 1.");
        let lines = vec![line];
        // Source order: Chapter 1 listed before Chapter 11 (common in TOCs).
        let extracted = vec![
            ExtractedLink {
                url: "#chapter-1".into(),
                label: "Chapter 1".into(),
            },
            ExtractedLink {
                url: "#chapter-11".into(),
                label: "Chapter 11".into(),
            },
        ];
        let links = build_link_hitboxes(&lines, &extracted);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].url, "#chapter-1");
        assert_eq!(links[0].label, "Chapter 1");
        assert_eq!(links[0].hitboxes[0].col_start, 19);
        assert_eq!(links[1].url, "#chapter-11");
        assert_eq!(links[1].label, "Chapter 11");
        assert_eq!(links[1].hitboxes[0].col_start, 4);
    }

    #[test]
    fn find_whole_needle_rejects_prefix() {
        let hay = "Chapter 11";
        assert_eq!(
            find_whole_needle(hay, "Chapter 1", 0),
            None
        );
        assert_eq!(
            find_whole_needle(hay, "Chapter 11", 0),
            Some(0)
        );
    }

    #[test]
    fn find_link_at_position() {
        let links = vec![MarkdownLink {
            url: "a.md".into(),
            label: "go".into(),
            hitboxes: vec![LinkHitbox {
                doc_line: 2,
                col_start: 4,
                col_end: 6,
            }],
        }];
        assert_eq!(find_link_at(&links, 2, 5), Some(0));
        assert_eq!(find_link_at(&links, 2, 3), None);
    }

    #[test]
    fn scroll_to_show_line_no_op_when_visible() {
        let mut md = build_markdown_viewer_state("x".into(), String::new());
        md.area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 22,
        };
        md.lines = (0..50).map(|i| Line::from(format!("line {i}"))).collect();
        md.scroll = 30;
        md.scroll_to_show_line(45);
        assert_eq!(
            md.scroll, 30,
            "visible link must not move scroll"
        );
    }

    #[test]
    fn scroll_to_show_line_below_viewport() {
        let mut md = build_markdown_viewer_state("x".into(), String::new());
        md.area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 22,
        };
        md.lines = (0..50).map(|i| Line::from(format!("line {i}"))).collect();
        md.scroll = 10;
        md.scroll_to_show_line(40);
        let vh = md.content_h() as usize;
        assert!(
            md.is_line_visible(40),
            "line 40 should be visible after scroll"
        );
        assert_eq!(
            md.scroll as usize,
            40usize.saturating_sub(vh - 1)
        );
    }
}
