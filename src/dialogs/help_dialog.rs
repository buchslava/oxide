//! F1 "Help" dialog. Scrollable shortcut reference with a narrow scrollbar; Esc, q, or click outside closes.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::terminal::size;
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::events::AppAction;
use crate::app::state::AppState;
use crate::ui::theme::DialogPalette;

/// F1 Help: scroll offset in lines (top visible line index).
#[derive(Debug, Clone, Default)]
pub struct HelpDialogState {
    pub scroll: usize,
}

/// Open the Help dialog (scroll reset).
pub fn open(app: &mut AppState) {
    app.help_dialog = Some(HelpDialogState::default());
}

/// Close the Help dialog.
pub fn close(app: &mut AppState) {
    app.help_dialog = None;
}

const SCROLLBAR_W: u16 = 1;
const HINT_H: u16 = 1;
const WHEEL_LINES: usize = 3;

/// Bounding box of the Help modal (must match [`draw`] and [`help_layout`]).
pub fn dialog_rect(area: Rect) -> Rect {
    let margin = 4u16;
    let max_w = area.width.saturating_sub(margin);
    let max_h = area.height.saturating_sub(margin);
    let w = max_w.min(122);
    let h = max_h.min(60).max(16);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Layout for the help modal: outer dialog rect, text body (no scrollbar), scrollbar column.
pub fn help_layout(area: Rect) -> (Rect, Rect, Rect) {
    let rect = dialog_rect(area);
    let inner = rect.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let body_h = inner.height.saturating_sub(HINT_H);
    let text = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width.saturating_sub(SCROLLBAR_W),
        height: body_h,
    };
    let scrollbar = Rect {
        x: inner.x + inner.width.saturating_sub(SCROLLBAR_W),
        y: inner.y,
        width: SCROLLBAR_W,
        height: body_h,
    };
    (rect, text, scrollbar)
}

fn viewport_rows(area: Rect) -> usize {
    help_layout(area).1.height as usize
}

fn help_max_scroll(
    area: Rect,
    total_lines: usize,
) -> usize {
    let vis = viewport_rows(area).max(1);
    total_lines.saturating_sub(vis)
}

fn clamp_help_scroll(
    scroll: usize,
    area: Rect,
    total_lines: usize,
) -> usize {
    scroll.min(help_max_scroll(area, total_lines))
}

/// Handle keyboard when the help dialog is open.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    _modifiers: KeyModifiers,
) -> Option<AppAction> {
    let state = app.help_dialog.as_mut()?;
    let area = term_area();
    let total = help_build_lines(&app.ui_palette.dialog).len();

    match code {
        KeyCode::Esc => Some(AppAction::HelpClose),
        KeyCode::Char(c) if c == 'q' => Some(AppAction::HelpClose),
        KeyCode::Up | KeyCode::Char('k') => {
            state.scroll = state.scroll.saturating_sub(1);
            state.scroll = clamp_help_scroll(state.scroll, area, total);
            Some(AppAction::Continue)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.scroll = (state.scroll + 1).min(help_max_scroll(area, total));
            Some(AppAction::Continue)
        }
        KeyCode::PageUp => {
            let step = viewport_rows(area).max(1);
            state.scroll = state.scroll.saturating_sub(step);
            state.scroll = clamp_help_scroll(state.scroll, area, total);
            Some(AppAction::Continue)
        }
        KeyCode::PageDown => {
            let step = viewport_rows(area).max(1);
            state.scroll = (state.scroll + step).min(help_max_scroll(area, total));
            Some(AppAction::Continue)
        }
        KeyCode::Home => {
            state.scroll = 0;
            Some(AppAction::Continue)
        }
        KeyCode::End => {
            state.scroll = help_max_scroll(area, total);
            Some(AppAction::Continue)
        }
        _ => None,
    }
}

/// Mouse: outside click closes; wheel scrolls inside dialog; click scrollbar jumps.
pub fn handle_mouse(
    app: &mut AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    let state = app.help_dialog.as_mut()?;
    let (dialog_rect, _text_rect, sb_rect) = help_layout(area);
    let (col, row) = (mouse_event.column, mouse_event.row);
    let total = help_build_lines(&app.ui_palette.dialog).len();
    let vis = viewport_rows(area).max(1);
    let max_scroll = help_max_scroll(area, total);

    match mouse_event.kind {
        MouseEventKind::ScrollUp => {
            if crate::ui::dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                state.scroll = state.scroll.saturating_sub(WHEEL_LINES);
                state.scroll = clamp_help_scroll(state.scroll, area, total);
            }
            Some(AppAction::Continue)
        }
        MouseEventKind::ScrollDown => {
            if crate::ui::dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                state.scroll = (state.scroll + WHEEL_LINES).min(max_scroll);
            }
            Some(AppAction::Continue)
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if !crate::ui::dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                return Some(AppAction::HelpClose);
            }
            if max_scroll > 0 && crate::ui::dialog_layout::pointer_in_dialog(col, row, sb_rect) {
                let rel = (row.saturating_sub(sb_rect.y)) as usize;
                if vis > 1 {
                    state.scroll = (rel * max_scroll / (vis - 1)).min(max_scroll);
                } else {
                    state.scroll = max_scroll;
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

/// Section title: left marker + bold heading.
fn help_h(
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

fn help_spacer() -> Line<'static> {
    Line::from("")
}

fn help_muted(
    d: &DialogPalette,
    text: &'static str,
) -> Line<'static> {
    Line::from(vec![Span::styled(
        text,
        Style::default().fg(d.help_dim),
    )])
}

fn help_build_lines(d: &DialogPalette) -> Vec<Line<'static>> {
    let key = d.help_key;
    let body = d.help_body;
    let dim = d.help_dim;

    let k = |s: &'static str| {
        Span::styled(
            s,
            Style::default().fg(key).add_modifier(Modifier::BOLD),
        )
    };
    let t = |s: &'static str| Span::raw(s);

    vec![
        help_h(d, "Overview"),
        Line::from(vec![
            t("    Oxide — two-panel file manager. Config: "),
            k("~/.oxide/settings.json"),
            t("."),
        ]),
        help_muted(
            d,
            "    Refresh (Ctrl+R, or Ctrl+X then R) re-reads the listing; if the folder vanished, the panel climbs to a valid parent.",
        ),
        help_spacer(),
        help_h(d, "Panels & navigation"),
        Line::from(vec![
            t("    "),
            k("↑ ↓"),
            t("  "),
            k("PgUp"),
            t("/"),
            k("PgDn"),
            t("     Move in list      "),
            k("← →"),
            t("   Move between columns"),
        ]),
        Line::from(vec![
            t("    "),
            k("Tab"),
            t("                  Switch active panel"),
        ]),
        Line::from(vec![
            t("    "),
            k("Enter"),
            t("                Open directory or run file"),
        ]),
        Line::from(vec![
            t("    "),
            k("Space"),
            t("                Mark      "),
            k("*"),
            t("  Invert selection"),
        ]),
        Line::from(vec![
            t("    "),
            k("+"),
            t(" / "),
            k("−"),
            t("              Mark / unmark by glob (same rules as Find; "),
            k("F9"),
            t(" pattern mode)."),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+X H"),
            Span::styled(
                "  Toggle hidden files (dot names). Dot entries use a dimmer list color when shown.",
                Style::default().fg(body),
            ),
        ]),
        Line::from(vec![
            t("    "),
            Span::styled(
                "    Mouse: click files, wheel scrolls lists; wheel scrolls F3/F4/diff when those are open.",
                Style::default().fg(body),
            ),
        ]),
        help_spacer(),
        help_h(d, "Command line"),
        Line::from(vec![
            t("    Type a character → focus command line and insert it. "),
            k("Enter"),
            t(" runs the shell command."),
        ]),
        Line::from(vec![
            t("    "),
            k("F12"),
            t("  Insert selected file name at cursor (no run). "),
            k("Tab"),
            t(" / "),
            k("Esc"),
            t("  Return focus to panels."),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+O"),
            t("  Subshell   "),
            k("Ctrl+R"),
            t("  Refresh active panel   "),
            k("Ctrl+C"),
            t(" / "),
            k("Ctrl+V"),
            t("  Copy / paste the command line"),
        ]),
        help_spacer(),
        help_h(d, "Function keys"),
        Line::from(vec![
            t("    "),
            k("F1"),
            t("  Help      "),
            k("F2"),
            t("  Rename / attrs   "),
            k("F3"),
            t("  View      "),
            k("F4"),
            t("  Edit"),
        ]),
        Line::from(vec![
            t("    "),
            k("F5"),
            t("  Copy      "),
            k("F6"),
            t("  Move      "),
            k("F7"),
            t("  New dir   "),
            k("F8"),
            t("  Delete"),
        ]),
        Line::from(vec![
            t("    "),
            k("F9"),
            t("  Settings (themes, panels, safe delete, …)   "),
            k("F10"),
            t("  Quit"),
        ]),
        help_spacer(),
        help_h(d, "Themes (F9 → Theme)"),
        Line::from(vec![
            t("    Built-in color presets: "),
            k("Oxide"),
            t(", "),
            k("Commander"),
            t(", "),
            k("Orange tradition"),
            t(", "),
            k("Breeze nostalgia"),
            t(", "),
            k("Neo's dream"),
            t(", "),
            k("Cosmos"),
            t(". Choice is saved to settings."),
        ]),
        help_spacer(),
        help_h(d, "Settings (F9)"),
        help_muted(
            d,
            "    Tab / Shift+Tab: sections list ↔ details. ↑↓ in General cycles rows; Space/Enter toggles.",
        ),
        Line::from(vec![
            t("    "),
            k("General"),
            Span::styled(
                ": autosave, shell cwd sync, auto-reopen after command, Find/+− pattern mode, safe delete.",
                Style::default().fg(body),
            ),
        ]),
        Line::from(vec![
            t("    "),
            k("Left / Right panel"),
            Span::styled(": view (one/two columns), sort, folders first, show hidden.", Style::default().fg(body)),
        ]),
        Line::from(vec![
            t("    "),
            k("Safe delete"),
            Span::styled(
                " — On (default): F8 uses OS trash when available. ZIP/tar panels: delete removes inside archive only.",
                Style::default().fg(body),
            ),
        ]),
        help_spacer(),
        help_h(d, "Shortcuts"),
        Line::from(vec![
            t("    "),
            k("Ctrl+X C"),
            t("  Save paths & panel to settings (configuration)   "),
            k("Ctrl+R"),
            t("  Refresh   "),
            k("Ctrl+X R"),
            t("  same (chord)"),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+X T"),
            t("  Column layout   "),
            k("Ctrl+X 1"),
            t(" / "),
            k("Ctrl+X 2"),
            t("  Left / right panel settings overlay"),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+O"),
            t("  Subshell   "),
            k("Ctrl+X O"),
            t("  same   "),
            k("Ctrl+X S"),
            t("  Size of selection   "),
            k("Ctrl+X D"),
            t("  Diff (see below)"),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+X F"),
            t("  Find file   "),
            k("Ctrl+X A"),
            t("  Archive   "),
            k("Ctrl+X N"),
            t("  New file"),
        ]),
        help_spacer(),
        help_h(d, "Copy / move / delete (F5–F8)"),
        Line::from(vec![
            Span::styled(
                "    Overwrite prompts, error recovery, and progress overlays. Delete confirms before run.",
                Style::default().fg(body),
            ),
        ]),
        help_spacer(),
        help_h(d, "Find file (Ctrl+X F)"),
        Line::from(vec![
            Span::styled(
                "    Start directory, file pattern, optional ignore & optional content search.",
                Style::default().fg(body),
            ),
        ]),
        Line::from(vec![
            t("    Pattern mode in "),
            k("F9"),
            t(": wildcards ("),
            k("* ?"),
            t(") or regex. Wildcards: "),
            k("|"),
            t(" separates alternative globs. Regex: "),
            k("|"),
            t(" is alternation (not split)."),
        ]),
        Line::from(vec![
            Span::styled(
                "    Ignore matches path relative to start (forward slashes). Tab/↑↓ fields; Enter search or chdir.",
                Style::default().fg(body),
            ),
        ]),
        Line::from(vec![
            t("    On a result: "),
            k("F3"),
            t(" view · "),
            k("F4"),
            t(" edit (dialog stays open)."),
        ]),
        help_spacer(),
        help_h(d, "Archives"),
        Line::from(vec![
            Span::styled(
                "    Open .zip, .tar.gz, .tgz as virtual folders (operations where supported). ",
                Style::default().fg(body),
            ),
            k("Ctrl+X A"),
            Span::styled(" creates an archive; extension picks format.", Style::default().fg(body)),
        ]),
        help_spacer(),
        help_h(d, "Compare (Ctrl+X D)"),
        Line::from(vec![
            Span::styled(
                "    Mark exactly two non-directory files (any panel). Order: left list marks first, then right.",
                Style::default().fg(body),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "    No marks: compares both panel directories; ",
                Style::default().fg(body),
            ),
            k("C"),
            Span::styled(" / ", Style::default().fg(body)),
            k("S"),
            Span::styled(" / ", Style::default().fg(body)),
            k("X"),
            Span::styled(" prefixes in lists. Side-by-side diff, aligned scroll, line + char highlights.", Style::default().fg(body)),
        ]),
        help_spacer(),
        help_h(d, "Viewer (F3)"),
        Line::from(vec![
            t("    "),
            k("Esc"),
            t(" close   "),
            k("H"),
            t(" hex/text   "),
            k("↑↓"),
            t(" "),
            k("PgUp/Dn"),
            t("   "),
            k("Home"),
            t("/"),
            k("End"),
            t("   wheel scrolls"),
        ]),
        help_muted(
            d,
            "    Large files load in background; non-printable text shown as “.” in text mode.",
        ),
        help_spacer(),
        help_h(d, "Diff viewer"),
        Line::from(vec![
            t("    "),
            k("Esc"),
            t(" close   "),
            k("↑↓"),
            t(" "),
            k("PgUp/Dn"),
            t(" "),
            k("Home"),
            t("/"),
            k("End"),
            t("   wheel — both panes stay aligned."),
        ]),
        help_spacer(),
        help_h(d, "Editor (F4)"),
        Line::from(vec![
            t("    "),
            k("F2"),
            t(" save   "),
            k("Esc"),
            t(" exit   "),
            k("Ctrl+X F"),
            t(" find   "),
            k("Shift+arrows"),
            t(" select   "),
            k("F3"),
            t(" line numbers   "),
            k("Ctrl+C/V"),
            t(" copy/paste"),
        ]),
        help_muted(
            d,
            "    Mouse wheel scrolls the buffer; the terminal caret stays hidden while the edit caret is off-screen, then returns when you move it back into view.",
        ),
        help_spacer(),
        help_h(d, "Other dialogs"),
        Line::from(vec![
            Span::styled(
                "    Tab / ↑↓ focus, Enter / Space confirm, Esc cancel (rename, mkdir, archive, pattern pick, …).",
                Style::default().fg(body),
            ),
        ]),
        help_spacer(),
        Line::from(vec![Span::styled(
            "  Click the scrollbar track to jump; drag is not used.",
            Style::default().fg(dim).add_modifier(Modifier::ITALIC),
        )]),
        Line::from(vec![Span::styled(
            "  Esc, q, or click outside this window to close.",
            Style::default().fg(dim).add_modifier(Modifier::ITALIC),
        )]),
    ]
}

fn render_help_scrollbar(
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
    let max_scroll = total.saturating_sub(vis);
    let track_st = Style::default().fg(d.text_muted);
    let thumb_st = Style::default().fg(d.border).add_modifier(Modifier::BOLD);

    if max_scroll == 0 {
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
    let thumb_top = scroll.saturating_mul(vis.saturating_sub(thumb_h)) / max_scroll;

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

/// Draw the Help dialog: dim layer is painted by the renderer before this.
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    let Some(state) = app.help_dialog.as_mut() else {
        return;
    };
    let area = f.area();
    let rect = dialog_rect(area);
    let d = &app.ui_palette.dialog;
    let dialog_bg = d.dialog_bg;
    let fill_style = Style::default().bg(dialog_bg).fg(d.text);
    let border_style = d.border_block_style();

    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Help ", border_style))
        .style(border_style);
    f.render_widget(block, rect);

    let inner = rect.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let (_dialog_rect, text_rect, sb_rect) = help_layout(area);
    let lines = help_build_lines(d);
    let total = lines.len();
    state.scroll = clamp_help_scroll(state.scroll, area, total);

    let visible: Vec<Line> = lines
        .into_iter()
        .skip(state.scroll)
        .take(text_rect.height as usize)
        .collect();

    let para = Paragraph::new(visible)
        .style(fill_style)
        .wrap(Wrap { trim: true });
    f.render_widget(para, text_rect);

    render_help_scrollbar(f, sb_rect, d, state.scroll, total);

    let hint_y = inner.y + inner.height.saturating_sub(HINT_H);
    let hint_rect = Rect {
        x: inner.x,
        y: hint_y,
        width: inner.width,
        height: HINT_H,
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            " ↑↓ j/k  PgUp/Dn  Home/End  wheel  scroll  ·  Esc  q  close  ·  outside click closes ",
            Style::default().bg(dialog_bg).fg(d.text_muted),
        ))
        .alignment(Alignment::Center),
        hint_rect,
    );
}
