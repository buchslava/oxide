//! F1 Actions: scrollable list of shortcuts as clickable rows (same layout as legacy Help).

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::terminal::size;
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::events::AppAction;
use crate::app::state::AppState;
use crate::ui::dialog_layout;
use crate::ui::theme::DialogPalette;

/// Row-local choice: maps to [`AppAction`] (keeps hit list `Copy` — `AppAction` is not).
#[derive(Debug, Clone, Copy)]
enum ActionsChoice {
    Suspend,
    RefreshBothPanels,
    RefreshActivePanel,
    ToggleShowHidden,
    ViewModeToggled,
    OpenLeftPanelSettings,
    OpenRightPanelSettings,
    PersistPanelState,
    OpenFindDialog,
    OpenDiffViewer,
    OpenSizeInfoDialog,
    OpenArchiveDialog,
    OpenNewFileDialog,
    CommandLineCopy,
    CommandLinePaste,
}

impl ActionsChoice {
    fn to_app_action(self) -> AppAction {
        match self {
            Self::Suspend => AppAction::Suspend,
            Self::RefreshBothPanels => AppAction::RefreshBothPanels,
            Self::RefreshActivePanel => AppAction::RefreshActivePanel,
            Self::ToggleShowHidden => AppAction::ToggleShowHidden,
            Self::ViewModeToggled => AppAction::ViewModeToggled,
            Self::OpenLeftPanelSettings => AppAction::OpenLeftPanelSettings,
            Self::OpenRightPanelSettings => AppAction::OpenRightPanelSettings,
            Self::PersistPanelState => AppAction::PersistPanelState,
            Self::OpenFindDialog => AppAction::OpenFindDialog,
            Self::OpenDiffViewer => AppAction::OpenDiffViewer,
            Self::OpenSizeInfoDialog => AppAction::OpenSizeInfoDialog,
            Self::OpenArchiveDialog => AppAction::OpenArchiveDialog,
            Self::OpenNewFileDialog => AppAction::OpenNewFileDialog,
            Self::CommandLineCopy => AppAction::CommandLineCopy,
            Self::CommandLinePaste => AppAction::CommandLinePaste,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActionsDialogState {
    pub scroll: usize,
}

pub fn open(app: &mut AppState) {
    app.actions_dialog = Some(ActionsDialogState::default());
}

pub fn close(app: &mut AppState) {
    app.actions_dialog = None;
}

const WHEEL_LINES: usize = 3;
/// Grey “button” is only the chord label (padded to this width for alignment).
const ACTION_ROW_BG: Color = Color::DarkGray;
/// Visual indent before the chord button (spaces).
const CHORD_BTN_INDENT_COLS: u16 = 4;

/// Chord strings for width alignment — keep in sync with every `action_line(..., chord, ...)` call.
const ACTION_CHORDS: &[&str] = &[
    "Ctrl+O  or  Ctrl+X O",
    "Ctrl+R",
    "Ctrl+X R",
    "Ctrl+X H",
    "Ctrl+X T",
    "Ctrl+X 1",
    "Ctrl+X 2",
    "Ctrl+X C",
    "Ctrl+X F",
    "Ctrl+X D",
    "Ctrl+X S",
    "Ctrl+X A",
    "Ctrl+X N",
    "Ctrl+C",
    "Ctrl+V",
];

fn chord_button_inner_cols() -> usize {
    ACTION_CHORDS
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(8)
        + 1
}

fn spaces_upto(n: usize) -> &'static str {
    const BUF: &str = "                                        ";
    &BUF[..n.min(BUF.len())]
}

fn viewport_rows(area: Rect) -> usize {
    dialog_layout::scroll_reference_modal_layout(area).1.height as usize
}

fn max_scroll(
    area: Rect,
    total_lines: usize,
) -> usize {
    let vis = viewport_rows(area).max(1);
    total_lines.saturating_sub(vis)
}

fn clamp_scroll(
    scroll: usize,
    area: Rect,
    total_lines: usize,
) -> usize {
    scroll.min(max_scroll(area, total_lines))
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

fn section_title(
    d: &DialogPalette,
    title: &'static str,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "  ▸ ",
            Style::default().fg(d.help_section_marker),
        ),
        Span::styled(
            title,
            Style::default()
                .fg(d.help_heading)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn muted_line(
    d: &DialogPalette,
    text: &'static str,
) -> Line<'static> {
    Line::from(vec![Span::styled(
        text,
        Style::default().fg(d.help_dim),
    )])
}

/// One action row: grey background only on the chord; description uses normal dialog background.
fn action_line(
    d: &DialogPalette,
    chord: &'static str,
    description: &'static str,
    inner_w: usize,
) -> Line<'static> {
    let pad = inner_w.saturating_sub(chord.chars().count());
    Line::from(vec![
        Span::raw("    "),
        Span::styled(
            chord,
            Style::default()
                .bg(ACTION_ROW_BG)
                .fg(d.help_key)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            spaces_upto(pad),
            Style::default().bg(ACTION_ROW_BG),
        ),
        Span::raw("  "),
        Span::styled(description, Style::default().fg(d.help_body)),
    ])
}

/// (display lines, parallel hit map: `Some` = chord button on this line — click only on grey columns).
fn build_lines_and_hits(d: &DialogPalette) -> (Vec<Line<'static>>, Vec<Option<ActionsChoice>>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut hits: Vec<Option<ActionsChoice>> = Vec::new();

    let push_blank = |lines: &mut Vec<_>, hits: &mut Vec<_>| {
        lines.push(Line::from(""));
        hits.push(None);
    };

    let inner_w = chord_button_inner_cols();

    lines.push(muted_line(
        d,
        "    Click a grey shortcut to run it (panel focus). Esc, q, or outside click closes.",
    ));
    hits.push(None);
    push_blank(&mut lines, &mut hits);

    lines.push(section_title(d, "Shell & refresh"));
    hits.push(None);
    lines.push(action_line(
        d,
        "Ctrl+O  or  Ctrl+X O",
        "Subshell (return with Ctrl+O or Ctrl+X then O)",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::Suspend));
    lines.push(action_line(
        d,
        "Ctrl+R",
        "Refresh both panels",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::RefreshBothPanels));
    lines.push(action_line(
        d,
        "Ctrl+X R",
        "Refresh active panel only",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::RefreshActivePanel));
    push_blank(&mut lines, &mut hits);

    lines.push(section_title(d, "Panels & layout"));
    hits.push(None);
    lines.push(action_line(
        d,
        "Ctrl+X H",
        "Toggle hidden files (dot names)",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::ToggleShowHidden));
    lines.push(action_line(
        d,
        "Ctrl+X T",
        "Toggle column layout (one / two columns)",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::ViewModeToggled));
    lines.push(action_line(
        d,
        "Ctrl+X 1",
        "Left panel settings overlay",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::OpenLeftPanelSettings));
    lines.push(action_line(
        d,
        "Ctrl+X 2",
        "Right panel settings overlay",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::OpenRightPanelSettings));
    lines.push(action_line(
        d,
        "Ctrl+X C",
        "Save paths and active panel to settings",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::PersistPanelState));
    push_blank(&mut lines, &mut hits);

    lines.push(section_title(d, "Files & tools"));
    hits.push(None);
    lines.push(action_line(d, "Ctrl+X F", "Find file", inner_w));
    hits.push(Some(ActionsChoice::OpenFindDialog));
    lines.push(action_line(
        d,
        "Ctrl+X D",
        "Diff (two marked files, or compare panel dirs if none marked)",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::OpenDiffViewer));
    lines.push(action_line(
        d,
        "Ctrl+X S",
        "Total size of selected items",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::OpenSizeInfoDialog));
    lines.push(action_line(
        d,
        "Ctrl+X A",
        "Create archive from selection",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::OpenArchiveDialog));
    lines.push(action_line(d, "Ctrl+X N", "New empty file", inner_w));
    hits.push(Some(ActionsChoice::OpenNewFileDialog));
    push_blank(&mut lines, &mut hits);

    lines.push(section_title(d, "Command line"));
    hits.push(None);
    lines.push(action_line(
        d,
        "Ctrl+C",
        "Copy command line (or clear if empty)",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::CommandLineCopy));
    lines.push(action_line(
        d,
        "Ctrl+V",
        "Paste into command line",
        inner_w,
    ));
    hits.push(Some(ActionsChoice::CommandLinePaste));

    (lines, hits)
}

pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    _modifiers: KeyModifiers,
) -> Option<AppAction> {
    let state = app.actions_dialog.as_mut()?;
    let area = term_area();
    let (lines, _) = build_lines_and_hits(&app.ui_palette.dialog);
    let total = lines.len();

    match code {
        KeyCode::Esc => Some(AppAction::ActionsClose),
        KeyCode::Char(c) if c == 'q' => Some(AppAction::ActionsClose),
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
    let state = app.actions_dialog.as_mut()?;
    let d = &app.ui_palette.dialog;
    let (dialog_rect, text_rect, sb_rect) = dialog_layout::scroll_reference_modal_layout(area);
    let (col, row) = (mouse_event.column, mouse_event.row);
    let (lines, hits) = build_lines_and_hits(d);
    let total = lines.len();
    let vis = viewport_rows(area).max(1);
    let max_s = max_scroll(area, total);

    match mouse_event.kind {
        MouseEventKind::ScrollUp => {
            if dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                state.scroll = state.scroll.saturating_sub(WHEEL_LINES);
                state.scroll = clamp_scroll(state.scroll, area, total);
            }
            Some(AppAction::Continue)
        }
        MouseEventKind::ScrollDown => {
            if dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                state.scroll = (state.scroll + WHEEL_LINES).min(max_s);
            }
            Some(AppAction::Continue)
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if !dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                return Some(AppAction::ActionsClose);
            }
            if max_s > 0 && dialog_layout::pointer_in_dialog(col, row, sb_rect) {
                let rel = (row.saturating_sub(sb_rect.y)) as usize;
                if vis > 1 {
                    state.scroll = (rel * max_s / (vis - 1)).min(max_s);
                } else {
                    state.scroll = max_s;
                }
                return Some(AppAction::Continue);
            }
            if dialog_layout::pointer_in_dialog(col, row, text_rect) {
                let rel_row = (row.saturating_sub(text_rect.y)) as usize;
                if rel_row < text_rect.height as usize {
                    let idx = state.scroll + rel_row;
                    if let Some(Some(choice)) = hits.get(idx) {
                        let rel_col = col.saturating_sub(text_rect.x);
                        let inner = chord_button_inner_cols() as u16;
                        let btn_end = CHORD_BTN_INDENT_COLS.saturating_add(inner);
                        if rel_col >= CHORD_BTN_INDENT_COLS && rel_col < btn_end {
                            app.actions_dialog = None;
                            return Some(choice.to_app_action());
                        }
                    }
                }
            }
            Some(AppAction::Continue)
        }
        _ => Some(AppAction::Continue),
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
    if vis == 0 || sb.width == 0 {
        return;
    }
    let max_s = total.saturating_sub(vis);
    let track_st = Style::default().fg(d.text_muted);
    let thumb_st = Style::default().fg(d.border).add_modifier(Modifier::BOLD);

    if max_s == 0 {
        for row in 0..vis {
            let y = sb.y.saturating_add(row as u16);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("│", track_st))),
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
    let Some(state) = app.actions_dialog.as_mut() else {
        return;
    };
    let area = f.area();
    let rect = dialog_layout::scroll_reference_modal_rect(area);
    let d = &app.ui_palette.dialog;
    let dialog_bg = d.dialog_bg;
    let fill_style = Style::default().bg(dialog_bg).fg(d.text);
    let border_style = d.border_block_style();

    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Actions ", border_style))
        .style(border_style);
    f.render_widget(block, rect);

    let inner = rect.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let (_dialog_rect, text_rect, sb_rect) = dialog_layout::scroll_reference_modal_layout(area);
    let (lines, _) = build_lines_and_hits(d);
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
            " ↑↓ j/k  PgUp/Dn  Home/End  wheel  scroll  ·  click grey shortcut  run  ·  Esc  q  close  ·  outside click closes ",
            Style::default().bg(dialog_bg).fg(d.text_muted),
        ))
        .alignment(Alignment::Center),
        hint_rect,
    );
}
