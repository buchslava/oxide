//! F1 Actions: scrollable list of shortcuts as clickable rows (same layout as legacy Help).
//! Tab / ↑↓ / j k move the focused row; Enter runs it. Ctrl+X chords and other palette shortcuts
//! still work (handled in [`crate::app::events::EventHandler`] before keys reach this module).

use std::sync::OnceLock;

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

#[derive(Debug, Clone)]
pub struct ActionsDialogState {
    pub scroll: usize,
    /// Index among rows that run an action (grey chord rows), not counting section headers.
    pub selected_action: usize,
}

impl Default for ActionsDialogState {
    fn default() -> Self {
        Self {
            scroll: 0,
            selected_action: 0,
        }
    }
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

/// Chord strings for width alignment — indices must match [`LayoutRow::Action`] `chord_i`.
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

const INTRO: &str = "    Grey shortcut or Enter runs the action (panel focus). Tab / ↑↓ / j k move focus. Esc, q, or outside click closes.";

/// Single source of truth for row order, hit targets, and chord indices into [`ACTION_CHORDS`].
#[derive(Clone, Copy)]
enum LayoutRow {
    Muted(&'static str),
    Blank,
    Section(&'static str),
    Action {
        chord_i: usize,
        desc: &'static str,
        choice: ActionsChoice,
    },
}

static ACTIONS_LAYOUT: &[LayoutRow] = &[
    LayoutRow::Muted(INTRO),
    LayoutRow::Blank,
    LayoutRow::Section("Shell & refresh"),
    LayoutRow::Action {
        chord_i: 0,
        desc: "Subshell (return with Ctrl+O or Ctrl+X then O)",
        choice: ActionsChoice::Suspend,
    },
    LayoutRow::Action {
        chord_i: 1,
        desc: "Refresh both panels",
        choice: ActionsChoice::RefreshBothPanels,
    },
    LayoutRow::Action {
        chord_i: 2,
        desc: "Refresh active panel only",
        choice: ActionsChoice::RefreshActivePanel,
    },
    LayoutRow::Blank,
    LayoutRow::Section("Panels & layout"),
    LayoutRow::Action {
        chord_i: 3,
        desc: "Toggle hidden files (dot names)",
        choice: ActionsChoice::ToggleShowHidden,
    },
    LayoutRow::Action {
        chord_i: 4,
        desc: "Toggle column layout (one / two columns)",
        choice: ActionsChoice::ViewModeToggled,
    },
    LayoutRow::Action {
        chord_i: 5,
        desc: "Left panel settings overlay",
        choice: ActionsChoice::OpenLeftPanelSettings,
    },
    LayoutRow::Action {
        chord_i: 6,
        desc: "Right panel settings overlay",
        choice: ActionsChoice::OpenRightPanelSettings,
    },
    LayoutRow::Action {
        chord_i: 7,
        desc: "Save paths and active panel to settings",
        choice: ActionsChoice::PersistPanelState,
    },
    LayoutRow::Blank,
    LayoutRow::Section("Files & tools"),
    LayoutRow::Action {
        chord_i: 8,
        desc: "Find file",
        choice: ActionsChoice::OpenFindDialog,
    },
    LayoutRow::Action {
        chord_i: 9,
        desc: "Diff (two marked files, or compare panel dirs if none marked)",
        choice: ActionsChoice::OpenDiffViewer,
    },
    LayoutRow::Action {
        chord_i: 10,
        desc: "Total size of selected items",
        choice: ActionsChoice::OpenSizeInfoDialog,
    },
    LayoutRow::Action {
        chord_i: 11,
        desc: "Create archive from selection",
        choice: ActionsChoice::OpenArchiveDialog,
    },
    LayoutRow::Action {
        chord_i: 12,
        desc: "New empty file",
        choice: ActionsChoice::OpenNewFileDialog,
    },
    LayoutRow::Blank,
    LayoutRow::Section("Command line"),
    LayoutRow::Action {
        chord_i: 13,
        desc: "Copy command line (or clear if empty)",
        choice: ActionsChoice::CommandLineCopy,
    },
    LayoutRow::Action {
        chord_i: 14,
        desc: "Paste into command line",
        choice: ActionsChoice::CommandLinePaste,
    },
];

static ACTION_HITS: OnceLock<Vec<Option<ActionsChoice>>> = OnceLock::new();

fn action_hits() -> &'static [Option<ActionsChoice>] {
    ACTION_HITS.get_or_init(|| {
        ACTIONS_LAYOUT
            .iter()
            .map(|row| match row {
                LayoutRow::Action { choice, .. } => Some(*choice),
                _ => None,
            })
            .collect()
    })
}

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

fn action_choice_count(hits: &[Option<ActionsChoice>]) -> usize {
    hits.iter().filter(|h| h.is_some()).count()
}

/// Line index of the `selected_action`‑th grey row (0‑based among action rows only).
fn line_index_for_selected_action(
    hits: &[Option<ActionsChoice>],
    selected_action: usize,
) -> Option<usize> {
    let mut remaining = selected_action;
    for (i, h) in hits.iter().enumerate() {
        if h.is_some() {
            if remaining == 0 {
                return Some(i);
            }
            remaining -= 1;
        }
    }
    None
}

fn sync_scroll_to_line(
    state: &mut ActionsDialogState,
    area: Rect,
    total_lines: usize,
    line: usize,
) {
    let vis = viewport_rows(area).max(1);
    if line < state.scroll {
        state.scroll = line;
    } else if line >= state.scroll + vis {
        state.scroll = line.saturating_sub(vis.saturating_sub(1));
    }
    state.scroll = clamp_scroll(state.scroll, area, total_lines);
}

fn apply_action_selection(
    state: &mut ActionsDialogState,
    new_sel: usize,
    hits: &[Option<ActionsChoice>],
    area: Rect,
    total_lines: usize,
    n_actions: usize,
) {
    state.selected_action = new_sel.min(n_actions - 1);
    if let Some(line) = line_index_for_selected_action(hits, state.selected_action) {
        sync_scroll_to_line(state, area, total_lines, line);
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
/// When `focused`, the whole row uses the dialog focus style (keyboard selection).
fn action_line(
    d: &DialogPalette,
    chord: &'static str,
    description: &'static str,
    inner_w: usize,
    focused: bool,
) -> Line<'static> {
    let pad = inner_w.saturating_sub(chord.chars().count());
    if focused {
        let st = d.focus_row_style().add_modifier(Modifier::BOLD);
        return Line::from(vec![
            Span::raw("    "),
            Span::styled(chord, st),
            Span::styled(spaces_upto(pad), st),
            Span::raw("  "),
            Span::styled(description, st),
        ]);
    }
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

fn build_lines(
    d: &DialogPalette,
    focus_line: Option<usize>,
) -> Vec<Line<'static>> {
    let inner_w = chord_button_inner_cols();
    let mut lines = Vec::with_capacity(ACTIONS_LAYOUT.len());
    let mut line_ix = 0usize;
    for row in ACTIONS_LAYOUT {
        let line = match *row {
            LayoutRow::Muted(text) => muted_line(d, text),
            LayoutRow::Blank => Line::from(""),
            LayoutRow::Section(title) => section_title(d, title),
            LayoutRow::Action {
                chord_i,
                desc,
                choice: _,
            } => {
                let chord = ACTION_CHORDS[chord_i];
                let focused = focus_line == Some(line_ix);
                action_line(d, chord, desc, inner_w, focused)
            }
        };
        lines.push(line);
        line_ix += 1;
    }
    lines
}

pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    let code = match code {
        KeyCode::Char('\t') => KeyCode::Tab,
        other => other,
    };
    if app.actions_dialog.is_none() {
        return None;
    }
    let area = term_area();
    let hits = action_hits();
    let total = hits.len();
    let n_actions = action_choice_count(hits);
    if n_actions == 0 {
        return match code {
            KeyCode::Esc => Some(AppAction::ActionsClose),
            KeyCode::Char(c) if c == 'q' => Some(AppAction::ActionsClose),
            _ => Some(AppAction::Continue),
        };
    }

    match code {
        KeyCode::Enter => {
            let selected = app
                .actions_dialog
                .as_ref()
                .map(|s| s.selected_action)?
                .min(n_actions - 1);
            let line = line_index_for_selected_action(hits, selected)?;
            let Some(choice) = hits.get(line).copied().flatten() else {
                return Some(AppAction::Continue);
            };
            app.actions_dialog = None;
            Some(choice.to_app_action())
        }
        KeyCode::Esc => Some(AppAction::ActionsClose),
        KeyCode::Char(c) if c == 'q' && !modifiers.contains(KeyModifiers::CONTROL) => {
            Some(AppAction::ActionsClose)
        }
        KeyCode::Tab => {
            let state = app.actions_dialog.as_mut()?;
            state.selected_action = state.selected_action.min(n_actions - 1);
            let next = (state.selected_action + 1) % n_actions;
            apply_action_selection(state, next, hits, area, total, n_actions);
            Some(AppAction::Continue)
        }
        KeyCode::BackTab => {
            let state = app.actions_dialog.as_mut()?;
            state.selected_action = state.selected_action.min(n_actions - 1);
            let prev = (state.selected_action + n_actions - 1) % n_actions;
            apply_action_selection(state, prev, hits, area, total, n_actions);
            Some(AppAction::Continue)
        }
        KeyCode::Up | KeyCode::Char('k')
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            let state = app.actions_dialog.as_mut()?;
            state.selected_action = state.selected_action.min(n_actions - 1);
            let prev = state.selected_action.saturating_sub(1);
            apply_action_selection(state, prev, hits, area, total, n_actions);
            Some(AppAction::Continue)
        }
        KeyCode::Down | KeyCode::Char('j')
            if !modifiers.contains(KeyModifiers::CONTROL) =>
        {
            let state = app.actions_dialog.as_mut()?;
            state.selected_action = state.selected_action.min(n_actions - 1);
            let next = (state.selected_action + 1).min(n_actions - 1);
            apply_action_selection(state, next, hits, area, total, n_actions);
            Some(AppAction::Continue)
        }
        KeyCode::PageUp => {
            let state = app.actions_dialog.as_mut()?;
            state.selected_action = state.selected_action.min(n_actions - 1);
            let step = viewport_rows(area).max(1);
            let prev = state.selected_action.saturating_sub(step);
            apply_action_selection(state, prev, hits, area, total, n_actions);
            Some(AppAction::Continue)
        }
        KeyCode::PageDown => {
            let state = app.actions_dialog.as_mut()?;
            state.selected_action = state.selected_action.min(n_actions - 1);
            let step = viewport_rows(area).max(1);
            let next = (state.selected_action + step).min(n_actions - 1);
            apply_action_selection(state, next, hits, area, total, n_actions);
            Some(AppAction::Continue)
        }
        KeyCode::Home => {
            let state = app.actions_dialog.as_mut()?;
            state.selected_action = state.selected_action.min(n_actions - 1);
            apply_action_selection(state, 0, hits, area, total, n_actions);
            Some(AppAction::Continue)
        }
        KeyCode::End => {
            let state = app.actions_dialog.as_mut()?;
            state.selected_action = state.selected_action.min(n_actions - 1);
            apply_action_selection(state, n_actions - 1, hits, area, total, n_actions);
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
    let (dialog_rect, text_rect, sb_rect) = dialog_layout::scroll_reference_modal_layout(area);
    let (col, row) = (mouse_event.column, mouse_event.row);
    let hits = action_hits();
    let total = hits.len();
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
    let hits = action_hits();
    let focus_line = line_index_for_selected_action(hits, state.selected_action);
    let lines = build_lines(d, focus_line);
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
            " Tab  ↑↓ j/k  PgUp/Dn  Home/End  wheel  scroll  ·  Enter  run focused  ·  click grey shortcut  run  ·  Esc  q  close  ·  outside click closes ",
            Style::default().bg(dialog_bg).fg(d.text_muted),
        ))
        .alignment(Alignment::Center),
        hint_rect,
    );
}
