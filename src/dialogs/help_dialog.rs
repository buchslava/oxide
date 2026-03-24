//! F1 "Help" dialog. Shows shortcut reference. Modal overlay; Esc, q, or click outside closes.

use crossterm::event::{KeyCode, KeyModifiers};
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

/// Open the Help dialog.
pub fn open(app: &mut AppState) {
    app.help_dialog = true;
}

/// Close the dialog.
pub fn close(app: &mut AppState) {
    app.help_dialog = false;
}

/// Handle a key when the help dialog is open.
pub fn handle_key(
    code: KeyCode,
    _modifiers: KeyModifiers,
) -> Option<AppAction> {
    match code {
        KeyCode::Esc => Some(AppAction::HelpClose),
        KeyCode::Char(c) if c == 'q' => Some(AppAction::HelpClose),
        _ => None,
    }
}

/// Section title: left marker + bold cyan heading.
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

/// Blank line between sections.
fn help_spacer() -> Line<'static> {
    Line::from("")
}

/// Muted body line (secondary description).
fn help_muted(
    d: &DialogPalette,
    text: &'static str,
) -> Line<'static> {
    Line::from(vec![Span::styled(
        text,
        Style::default().fg(d.help_dim),
    )])
}

fn help_lines(d: &DialogPalette) -> Vec<Line<'static>> {
    let key = d.help_key;
    let body = d.help_body;
    let dim = d.help_dim;

    let k = |s: &'static str| Span::styled(s, Style::default().fg(key).add_modifier(Modifier::BOLD));
    let t = |s: &'static str| Span::raw(s);

    vec![
        help_h(d, "Features"),
        Line::from(vec![
            t("    "),
            Span::styled("Dual panels", Style::default().fg(body)),
            t(" — full keyboard + mouse. "),
            k("F5–F8"),
            Span::styled(" copy, move, delete. ", Style::default().fg(body)),
            k("Ctrl+T"),
            Span::styled(" column layout.", Style::default().fg(body)),
        ]),
        Line::from(vec![
            t("    "),
            k("F3"),
            Span::styled(" viewer (text/hex) · ", Style::default().fg(body)),
            k("F4"),
            Span::styled(" editor · ", Style::default().fg(body)),
            k("F7"),
            Span::styled(" mkdir · ", Style::default().fg(body)),
            k("F2"),
            Span::styled(" rename & attributes.", Style::default().fg(body)),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+G"),
            Span::styled(" size · ", Style::default().fg(body)),
            k("Ctrl+H"),
            Span::styled(" hidden · ", Style::default().fg(body)),
            k("Ctrl+O"),
            Span::styled(" shell · disk space (Unix).", Style::default().fg(body)),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+F"),
            Span::styled(" find · ", Style::default().fg(body)),
            k("Ctrl+A"),
            Span::styled(" zip · ", Style::default().fg(body)),
            k("Ctrl+N"),
            Span::styled(" new file. Cmd line: ", Style::default().fg(body)),
            k("F12"),
            Span::styled(" inserts name.", Style::default().fg(body)),
        ]),
        help_muted(d, "    Large viewer files load in background; non-printable text shown as “.”"),
        help_spacer(),
        help_h(d, "Navigation"),
        Line::from(vec![
            t("    "),
            k("↑ ↓"),
            t("  PgUp / PgDn     Move in list"),
        ]),
        Line::from(vec![
            t("    "),
            k("← →"),
            t("                 Move between columns"),
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
            k("-"),
            t("              Mark / unmark by pattern  ("),
            k("F9"),
            t(": wildcards or regex; "),
            k("|"),
            t(" = multiple globs)"),
        ]),
        Line::from(vec![
            t("    Type a character → command line   "),
            k("Tab / Esc"),
            t("  Back to panels"),
        ]),
        help_spacer(),
        help_h(d, "Function keys"),
        Line::from(vec![
            t("    "),
            k("F1"),
            t("  Help      "),
            k("F2"),
            t("  Rename    "),
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
            t("  Settings  "),
            k("F10"),
            t(" Quit"),
        ]),
        help_spacer(),
        help_h(d, "Settings (F9) — General"),
        Line::from(vec![
            t("    "),
            k("Safe delete"),
            Span::styled(
                " — On (default): F8 moves to OS trash when supported. Off: permanent delete.",
                Style::default().fg(body),
            ),
        ]),
        help_muted(d, "    ZIP panels: entries removed inside the archive only. Trash N/A → option dimmed."),
        Line::from(vec![
            t("    "),
            Span::styled("Also:", Style::default().fg(body)),
            Span::styled(
                " autosave · shell sync · panel view/sort · ",
                Style::default().fg(body),
            ),
            k("file pattern"),
            Span::styled(" (wildcards vs regex for Find & +/−).", Style::default().fg(body)),
        ]),
        help_spacer(),
        help_h(d, "Shortcuts"),
        Line::from(vec![
            t("    "),
            k("Ctrl+O"),
            t("  Shell       "),
            k("Ctrl+H"),
            t("  Hidden    "),
            k("Ctrl+G"),
            t("  Size"),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+R"),
            t("  Refresh     "),
            k("Ctrl+T"),
            t("  Columns   "),
            k("Ctrl+Q/W"),
            t("  Panel settings"),
        ]),
        Line::from(vec![
            t("    "),
            k("Ctrl+F"),
            t("  Find        "),
            k("Ctrl+A"),
            t("  Archive   "),
            k("Ctrl+N"),
            t("  New file"),
        ]),
        help_spacer(),
        help_h(d, "Find file (Ctrl+F)"),
        Line::from(vec![
            t("    "),
            Span::styled(
                "Start dir, file pattern, optional ignore & content. Mode: ",
                Style::default().fg(body),
            ),
            k("F9"),
            Span::styled(" → ", Style::default().fg(body)),
            k("* ?"),
            Span::styled(" wildcards or regex.", Style::default().fg(body)),
        ]),
        Line::from(vec![
            t("    "),
            k("Wildcards"),
            Span::styled(": ", Style::default().fg(body)),
            k("|"),
            Span::styled(" separates globs (e.g. ", Style::default().fg(body)),
            k("a*|b?"),
            Span::styled("). ", Style::default().fg(body)),
            k("Regex"),
            Span::styled(": ", Style::default().fg(body)),
            k("|"),
            Span::styled(" is alternation (not split).", Style::default().fg(body)),
        ]),
        Line::from(vec![
            t("    "),
            k("Ignore"),
            Span::styled(
                ": same rules; matched on path relative to start — excluded if it matches",
                Style::default().fg(body),
            ),
            t(" ("),
            k("*.zip"),
            Span::styled(" + ignore ", Style::default().fg(body)),
            k("*node_modules*"),
            Span::styled(" …).", Style::default().fg(body)),
        ]),
        Line::from(vec![
            t("    "),
            k("Tab / ↑↓"),
            t("  Navigate   "),
            k("Enter"),
            t("  Search / chdir   "),
            k("Esc"),
            t("  Close"),
        ]),
        Line::from(vec![
            t("    Result: "),
            k("F3"),
            t(" view · "),
            k("F4"),
            t(" edit (dialog stays open)"),
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
            t(" scroll"),
        ]),
        help_spacer(),
        help_h(d, "Editor (F4)"),
        Line::from(vec![
            t("    "),
            k("F2"),
            t(" save   "),
            k("Esc"),
            t(" exit   "),
            k("Ctrl+F"),
            t(" find in file   "),
            k("Ctrl+C/V"),
            t(" copy/paste"),
        ]),
        help_spacer(),
        help_h(d, "Dialogs"),
        Line::from(vec![
            t("    "),
            k("Tab / ↑↓"),
            t("  focus   "),
            k("Enter"),
            t("  confirm   "),
            k("Esc"),
            t("  cancel"),
        ]),
        help_spacer(),
        Line::from(vec![Span::styled(
            "  Esc or click outside this window to close",
            Style::default().fg(dim).add_modifier(Modifier::ITALIC),
        )]),
    ]
}

/// Bounding box of the Help modal (must match [`draw`]).
pub fn dialog_rect(area: Rect) -> Rect {
    let margin = 4u16;
    let max_w = area.width.saturating_sub(margin);
    let max_h = area.height.saturating_sub(margin);
    let w = max_w.min(122);
    let h = max_h.min(56);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Draw the Help dialog as a modal: dimmed full screen, then dialog box on top.
pub fn draw(
    f: &mut Frame,
    app: &AppState,
) {
    if !app.help_dialog {
        return;
    }
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
    let hint_h = 1u16;
    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(hint_h),
    };

    let para = Paragraph::new(help_lines(d))
        .style(fill_style)
        .wrap(Wrap { trim: true });
    f.render_widget(para, content_rect);

    let hint_rect = Rect {
        x: inner.x,
        y: inner.y + content_rect.height,
        width: inner.width,
        height: hint_h,
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            " Esc  ·  q  close   ·   click outside to dismiss ",
            Style::default()
                .bg(dialog_bg)
                .fg(d.text_muted),
        ))
        .alignment(Alignment::Center),
        hint_rect,
    );
}
