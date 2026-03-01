//! F1 "Settings" dialog. Help / shortcut reference. Esc or mouse click closes.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app_state::AppState;
use crate::events::AppAction;

/// Open the Settings dialog.
pub fn open(app: &mut AppState) {
    app.settings_dialog = Some(());
}

/// Close the dialog.
pub fn close(app: &mut AppState) {
    app.settings_dialog = None;
}

/// Handle a key when the settings dialog is open.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    if app.settings_dialog.is_none() {
        return None;
    }
    match code {
        KeyCode::Esc => {
            close(app);
            return Some(AppAction::SettingsClose);
        }
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) && c == 'o' => {
            return Some(AppAction::Suspend);
        }
        _ => {}
    }
    Some(AppAction::Continue)
}

/// Help text lines for the Settings dialog.
fn help_lines() -> Vec<Line<'static>> {
    let cyan = Color::Cyan;
    vec![
        Line::from(vec![Span::styled("Panels", cyan), Span::raw(":")]),
        Line::from(vec![Span::raw("  ↑↓ PgUp/PgDn  Navigate   Tab  Switch panel")]),
        Line::from(vec![Span::raw("  ← →          Move col   Enter  Open dir / run")]),
        Line::from(vec![Span::raw("  Space  Mark   *  Invert selection")]),
        Line::from(""),
        Line::from(vec![Span::styled("F-keys", cyan), Span::raw(": F1 Settings  F2 Rename  F3 View  F4 Edit")]),
        Line::from(vec![Span::raw("  F5 Copy  F6 Move  F7 New dir  F8 Delete  F10 Quit")]),
        Line::from(""),
        Line::from(vec![Span::styled("Shortcuts", cyan), Span::raw(": Ctrl+O  Shell   Ctrl+H  Hidden   Ctrl+G  Size   Ctrl+R  Refresh   Ctrl+T  View")]),
        Line::from(vec![Span::raw("  Type char → command line   Tab/Esc → panel")]),
        Line::from(""),
        Line::from(vec![Span::styled("Editor", cyan), Span::raw(" (F4): Shift+←→↑↓ select  F3 lines  Ctrl+C/V  F2 Save  Esc exit")]),
        Line::from(vec![Span::raw("  Ctrl+F  Find in file")]),
        Line::from(""),
        Line::from(vec![Span::styled("Viewer", cyan), Span::raw(" (F3): Esc close  H  hex/text  ↑↓ PgUp/PgDn  scroll")]),
        Line::from(""),
        Line::from(vec![Span::styled("Create dir", cyan), Span::raw(" (F7): Tab  textarea↔buttons  Enter/Esc")]),
        Line::from(vec![Span::styled("Dialogs", cyan), Span::raw(": Tab/↑↓  choose   Enter  confirm   Esc  cancel")]),
    ]
}

/// Draw the Settings dialog: help / shortcut reference.
pub fn draw(f: &mut Frame, app: &mut AppState) {
    if app.settings_dialog.is_none() {
        return;
    }
    let area = f.area();
    const PAD_H: u16 = 2;
    let max_w = 62u16;
    let w = max_w.min(area.width.saturating_sub(4));
    let h = 24u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };
    let grey_bg = Color::Rgb(60, 60, 60);
    let fill_style = Style::default().bg(grey_bg).fg(Color::White);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Settings / Help ")
        .style(fill_style.fg(Color::Cyan));
    f.render_widget(block, rect);
    let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
    let content_h = inner.height.saturating_sub(2);
    let content = Rect {
        x: inner.x + PAD_H,
        y: inner.y,
        width: inner.width.saturating_sub(PAD_H * 2),
        height: content_h,
    };
    let para = Paragraph::new(help_lines())
        .style(fill_style)
        .wrap(Wrap { trim: true });
    f.render_widget(para, content);
    let hint_rect = Rect {
        x: inner.x,
        y: inner.y + content_h,
        width: inner.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new("Esc or click to close")
            .style(fill_style.fg(Color::DarkGray))
            .alignment(Alignment::Center),
        hint_rect,
    );
}
