//! F1 "Help" dialog. Shows shortcut reference. Modal overlay; Esc or mouse click closes.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app_state::AppState;
use crate::events::AppAction;

/// Open the Help dialog.
pub fn open(app: &mut AppState) {
    app.help_dialog = true;
}

/// Close the dialog.
pub fn close(app: &mut AppState) {
    app.help_dialog = false;
}

/// Handle a key when the help dialog is open.
pub fn handle_key(code: KeyCode, _modifiers: KeyModifiers) -> Option<AppAction> {
    match code {
        KeyCode::Esc => Some(AppAction::HelpClose),
        KeyCode::Char(c) if c == 'q' => Some(AppAction::HelpClose),
        _ => None,
    }
}

fn help_lines() -> Vec<Line<'static>> {
    let heading = Color::Cyan;
    let key = Color::Rgb(255, 200, 100);  // warm accent for keys
    let dim = Color::DarkGray;

    vec![
        Line::from(vec![Span::styled("  Navigation", heading), Span::raw(" — panels & command line")]),
        Line::from(""),
        Line::from(vec![Span::raw("    "), Span::styled("↑ ↓", key), Span::raw("  PgUp / PgDn    Move in list")]),
        Line::from(vec![Span::raw("    "), Span::styled("← →", key), Span::raw("                 Move one column")]),
        Line::from(vec![Span::raw("    "), Span::styled("Tab", key), Span::raw("                  Switch active panel")]),
        Line::from(vec![Span::raw("    "), Span::styled("Enter", key), Span::raw("                Open directory or run file")]),
        Line::from(vec![Span::raw("    "), Span::styled("Space", key), Span::raw("                Mark item   "), Span::styled("*", key), Span::raw("  Invert selection")]),
        Line::from(vec![Span::raw("    Type a key   Go to command line   "), Span::styled("Tab / Esc", key), Span::raw("  Back to panel")]),
        Line::from(""),
        Line::from(vec![Span::styled("  Function keys", heading)]),
        Line::from(""),
        Line::from(vec![Span::raw("    "), Span::styled("F1", key), Span::raw("  Help    "), Span::styled("F2", key), Span::raw("  Rename/attrs  "), Span::styled("F3", key), Span::raw("  View    "), Span::styled("F4", key), Span::raw("  Edit")]),
        Line::from(vec![Span::raw("    "), Span::styled("F5", key), Span::raw("  Copy    "), Span::styled("F6", key), Span::raw("  Move    "), Span::styled("F7", key), Span::raw("  New dir  "), Span::styled("F8", key), Span::raw("  Delete")]),
        Line::from(vec![Span::raw("    "), Span::styled("F9", key), Span::raw("  Settings      "), Span::styled("F10", key), Span::raw("  Quit")]),
        Line::from(""),
        Line::from(vec![Span::styled("  Shortcuts", heading)]),
        Line::from(""),
        Line::from(vec![Span::raw("    "), Span::styled("Ctrl+O", key), Span::raw("  Shell   "), Span::styled("Ctrl+H", key), Span::raw("  Toggle hidden   "), Span::styled("Ctrl+G", key), Span::raw("  Size of selection")]),
        Line::from(vec![Span::raw("    "), Span::styled("Ctrl+R", key), Span::raw("  Refresh   "), Span::styled("Ctrl+T", key), Span::raw("  One/two columns   "), Span::styled("Ctrl+Q/W", key), Span::raw("  Panel settings")]),
        Line::from(vec![Span::raw("    "), Span::styled("Ctrl+F", key), Span::raw("  Find file   "), Span::styled("Ctrl+A", key), Span::raw("  Archive (zip) selected")]),
        Line::from(vec![Span::raw("    "), Span::styled("Ctrl+N", key), Span::raw("  New file (in current dir or archive)")]),
        Line::from(""),
        Line::from(vec![Span::styled("  Find file", heading), Span::raw(" (Ctrl+F)")]),
        Line::from(""),
        Line::from(vec![Span::raw("    Set start dir, file pattern ("), Span::styled("* ?", key), Span::raw("), optional content pattern.")]),
        Line::from(vec![Span::raw("    "), Span::styled("Tab / ↑↓", key), Span::raw("  Move   "), Span::styled("Enter", key), Span::raw("  Start search or chdir to result   "), Span::styled("Esc", key), Span::raw("  Close")]),
        Line::from(vec![Span::raw("    On a result: "), Span::styled("F3", key), Span::raw(" View   "), Span::styled("F4", key), Span::raw(" Edit (dialog stays open)")]),
        Line::from(""),
        Line::from(vec![Span::styled("  Viewer", heading), Span::raw(" (F3)  — "), Span::styled("Esc", key), Span::raw(" close   "), Span::styled("H", key), Span::raw(" hex/text   "), Span::styled("↑↓", key), Span::raw(" scroll")]),
        Line::from(vec![Span::styled("  Editor", heading), Span::raw(" (F4)  — "), Span::styled("F2", key), Span::raw(" Save   "), Span::styled("Esc", key), Span::raw(" exit   "), Span::styled("Ctrl+F", key), Span::raw(" Find in file   "), Span::styled("Ctrl+C/V", key), Span::raw(" Copy/Paste")]),
        Line::from(""),
        Line::from(vec![Span::styled("  Dialogs", heading), Span::raw(" — "), Span::styled("Tab / ↑↓", key), Span::raw(" choose   "), Span::styled("Enter", key), Span::raw(" confirm   "), Span::styled("Esc", key), Span::raw(" cancel")]),
        Line::from(""),
        Line::from(vec![Span::styled("  Esc  or  click  anywhere  to  close  this  help", dim)]),
    ]
}

/// Draw the Help dialog as a modal: dimmed full screen, then dialog box on top.
pub fn draw(f: &mut Frame, app: &AppState) {
    if !app.help_dialog {
        return;
    }
    let area = f.area();

    // Modal: dim the entire screen so the dialog is clearly on top
    let dim_style = Style::default().bg(Color::Rgb(18, 18, 24)).fg(Color::DarkGray);
    let dim_block = Block::default().borders(Borders::NONE).style(dim_style);
    f.render_widget(dim_block, area);

    const MIN_W: u16 = 76;
    const MIN_H: u16 = 30;
    let w = MIN_W.min(area.width.saturating_sub(4));
    let h = MIN_H.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };

    let dialog_bg = Color::Rgb(36, 38, 42);
    let fill_style = Style::default().bg(dialog_bg).fg(Color::White);
    let border_style = Style::default()
        .bg(dialog_bg)
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Help ", border_style))
        .style(border_style);
    f.render_widget(block, rect);

    let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
    let hint_h = 1u16;
    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(hint_h),
    };

    let para = Paragraph::new(help_lines())
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
        Paragraph::new(Span::styled(" Esc  close  ·  click  anywhere  to  dismiss ", fill_style.fg(Color::DarkGray)))
            .alignment(Alignment::Center),
        hint_rect,
    );
}
