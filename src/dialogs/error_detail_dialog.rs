//! Scrollable error details (same geometry as F1 Actions). Esc / Enter / q / outside click closes.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::terminal::size;
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::events::AppAction;
use crate::app::state::AppState;
use crate::ui::dialog_layout;
use crate::ui::theme::DialogPalette;

/// Scrollable error popup (reuses help modal geometry).
#[derive(Debug, Clone)]
pub struct ErrorDetailState {
    pub title: String,
    pub body: String,
    pub scroll: usize,
}

const WHEEL_LINES: usize = 3;

fn layout(area: Rect) -> (Rect, Rect, Rect) {
    dialog_layout::scroll_reference_modal_layout(area)
}

fn viewport_rows(area: Rect) -> usize {
    layout(area).1.height as usize
}

fn max_scroll(
    area: Rect,
    total: usize,
) -> usize {
    let vis = viewport_rows(area).max(1);
    total.saturating_sub(vis)
}

fn clamp_scroll(
    scroll: usize,
    area: Rect,
    total: usize,
) -> usize {
    scroll.min(max_scroll(area, total))
}

fn wrap_paragraphs(
    text: &str,
    width: usize,
) -> Vec<Line<'static>> {
    let w = width.max(12);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        let mut current = String::new();
        for word in raw.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() > w {
                lines.push(Line::from(std::mem::take(&mut current)));
                current.push_str(word);
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            lines.push(Line::from(current));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from("(no message)"));
    }
    lines
}

fn build_lines(
    body: &str,
    area: Rect,
) -> Vec<Line<'static>> {
    let (_, text_rect, _) = layout(area);
    let w = text_rect.width.saturating_sub(1) as usize;
    wrap_paragraphs(body, w)
}

pub fn open(
    app: &mut AppState,
    title: impl Into<String>,
    body: impl Into<String>,
) {
    app.error_detail = Some(ErrorDetailState {
        title: title.into(),
        body: body.into(),
        scroll: 0,
    });
}

pub fn open_from_io(
    app: &mut AppState,
    title: &str,
    err: std::io::Error,
) {
    let mut body = err.to_string();
    if let Some(code) = err.raw_os_error() {
        body.push_str(&format!("\n\nOS error code: {code}"));
    }
    open(app, title, body);
}

pub fn close(app: &mut AppState) {
    app.error_detail = None;
}

pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    _modifiers: KeyModifiers,
) -> Option<AppAction> {
    let state = app.error_detail.as_mut()?;
    let area = term_area();
    let total = build_lines(&state.body, area).len();

    match code {
        KeyCode::Esc | KeyCode::Enter => Some(AppAction::ErrorDetailClose),
        KeyCode::Char(c) if c == 'q' => Some(AppAction::ErrorDetailClose),
        KeyCode::Up | KeyCode::Char('k') => {
            state.scroll = state.scroll.saturating_sub(1);
            state.scroll = clamp_scroll(state.scroll, area, total);
            Some(AppAction::Continue)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.scroll = (state.scroll + 1).min(max_scroll(area, total));
            Some(AppAction::Continue)
        }
        KeyCode::PageUp => {
            let step = viewport_rows(area).max(1);
            state.scroll = state.scroll.saturating_sub(step);
            state.scroll = clamp_scroll(state.scroll, area, total);
            Some(AppAction::Continue)
        }
        KeyCode::PageDown => {
            let step = viewport_rows(area).max(1);
            state.scroll = (state.scroll + step).min(max_scroll(area, total));
            Some(AppAction::Continue)
        }
        KeyCode::Home => {
            state.scroll = 0;
            Some(AppAction::Continue)
        }
        KeyCode::End => {
            state.scroll = max_scroll(area, total);
            Some(AppAction::Continue)
        }
        _ => None,
    }
}

pub fn handle_mouse(
    app: &mut AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    let state = app.error_detail.as_mut()?;
    let (dialog_rect, _text_rect, sb_rect) = layout(area);
    let (col, row) = (mouse_event.column, mouse_event.row);
    let total = build_lines(&state.body, area).len();
    let vis = viewport_rows(area).max(1);
    let max_s = max_scroll(area, total);

    match mouse_event.kind {
        MouseEventKind::ScrollUp => {
            if crate::ui::dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                state.scroll = state.scroll.saturating_sub(WHEEL_LINES);
                state.scroll = clamp_scroll(state.scroll, area, total);
            }
            Some(AppAction::Continue)
        }
        MouseEventKind::ScrollDown => {
            if crate::ui::dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                state.scroll = (state.scroll + WHEEL_LINES).min(max_s);
            }
            Some(AppAction::Continue)
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if !crate::ui::dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                return Some(AppAction::ErrorDetailClose);
            }
            if max_s > 0 && crate::ui::dialog_layout::pointer_in_dialog(col, row, sb_rect) {
                let rel = (row.saturating_sub(sb_rect.y)) as usize;
                if vis > 1 {
                    state.scroll = (rel * max_s / (vis - 1)).min(max_s);
                } else {
                    state.scroll = max_s;
                }
            }
            Some(AppAction::Continue)
        }
        _ => Some(AppAction::Continue),
    }
}

fn term_area() -> Rect {
    let (tw, th) = size().unwrap_or((80, 24));
    Rect {
        x: 0,
        y: 0,
        width: tw,
        height: th,
    }
}

fn render_scrollbar(
    f: &mut Frame,
    sb: Rect,
    d: &DialogPalette,
    scroll: usize,
    total: usize,
) {
    let vis = sb.height as usize;
    let max_s = total.saturating_sub(vis);
    let thumb_st = Style::default().fg(d.accent);
    let track_st = Style::default().fg(d.text_muted);
    if max_s == 0 {
        for row in 0..vis {
            let y = sb.y.saturating_add(row as u16);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("▒", track_st))),
                Rect {
                    x: sb.x,
                    y,
                    width: 1,
                    height: 1,
                },
            );
        }
        return;
    }
    let thumb_h = ((vis * vis + total - 1) / total).max(1).min(vis);
    let thumb_top = scroll.saturating_mul(vis.saturating_sub(thumb_h)) / max_s;
    for row in 0..vis {
        let is_thumb = row >= thumb_top && row < thumb_top + thumb_h;
        let ch = if is_thumb { "█" } else { "▒" };
        let st = if is_thumb { thumb_st } else { track_st };
        let y = sb.y.saturating_add(row as u16);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(ch, st))),
            Rect {
                x: sb.x,
                y,
                width: 1,
                height: 1,
            },
        );
    }
}

pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    let Some(state) = app.error_detail.as_mut() else {
        return;
    };
    let area = f.area();
    let rect = dialog_layout::scroll_reference_modal_rect(area);
    let d = &app.ui_palette.dialog;
    let dialog_bg = d.dialog_bg;
    let fill_style = Style::default().bg(dialog_bg).fg(d.text);
    let border_style = d.border_block_style();

    f.render_widget(Clear, rect);
    let title = format!(" {} ", state.title.trim());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, border_style))
        .style(border_style);
    f.render_widget(block, rect);

    let inner = rect.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let (_dr, text_rect, sb_rect) = layout(area);
    let lines = build_lines(&state.body, area);
    let total = lines.len();
    state.scroll = clamp_scroll(state.scroll, area, total);

    let visible: Vec<Line> = lines
        .into_iter()
        .skip(state.scroll)
        .take(text_rect.height as usize)
        .collect();

    let para = Paragraph::new(visible)
        .style(fill_style)
        .wrap(Wrap { trim: true });
    f.render_widget(para, text_rect);

    render_scrollbar(f, sb_rect, d, state.scroll, total);

    let hint_y = inner.y + inner.height.saturating_sub(dialog_layout::SCROLL_REFERENCE_MODAL_HINT_H);
    let hint_rect = Rect {
        x: inner.x,
        y: hint_y,
        width: inner.width,
        height: dialog_layout::SCROLL_REFERENCE_MODAL_HINT_H,
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            " ↑↓ j/k  PgUp/Dn  Home/End  wheel  scroll  ·  Esc  Enter  q  close  ·  outside click closes ",
            Style::default().bg(dialog_bg).fg(d.text_muted),
        ))
        .alignment(Alignment::Center),
        hint_rect,
    );
}
