//! F7 "Create a new Directory" dialog (MC-style). Single text field for the new folder name;
//! Enter = create (if non-empty), Esc = cancel.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app_state::AppState;
use crate::events::AppAction;
use crate::panel_backend;

/// State for F7 "Create a new Directory" dialog (MC-style). Single text field for the new folder name.
/// focus: 0 = textarea, 1 = Create, 2 = Cancel.
#[derive(Debug, Clone)]
pub struct MkdirDialogState {
    pub name: String,
    pub cursor: usize,
    pub focus: usize,
}

/// Open the dialog with an empty folder name.
pub fn open(app: &mut AppState) {
    open_with_name(app, String::new());
}

/// Open the dialog with an optional default name (e.g. from selected file).
pub fn open_with_name(app: &mut AppState, default_name: String) {
    let cursor = default_name.len();
    app.mkdir_dialog = Some(MkdirDialogState {
        name: default_name,
        cursor,
        focus: 0,
    });
}

/// Close the dialog without creating. Called on Esc or Ctrl+C.
pub fn cancel(app: &mut AppState) {
    app.mkdir_dialog = None;
}

/// Take the entered name and close the dialog. Returns the name (may be empty).
pub fn confirm(app: &mut AppState) -> Option<String> {
    app.mkdir_dialog.take().map(|d| d.name)
}

fn insert_char(app: &mut AppState, c: char) {
    if let Some(ref mut d) = app.mkdir_dialog {
        let at = d.cursor.min(d.name.len());
        d.name.insert(at, c);
        d.cursor = at + 1;
    }
}

fn backspace(app: &mut AppState) {
    if let Some(ref mut d) = app.mkdir_dialog {
        if d.cursor > 0 && d.cursor <= d.name.len() {
            d.name.remove(d.cursor - 1);
            d.cursor -= 1;
        }
    }
}

fn move_left(app: &mut AppState) {
    if let Some(ref mut d) = app.mkdir_dialog {
        if d.cursor > 0 {
            d.cursor -= 1;
        }
    }
}

fn move_right(app: &mut AppState) {
    if let Some(ref mut d) = app.mkdir_dialog {
        if d.cursor < d.name.len() {
            d.cursor += 1;
        }
    }
}

/// Create the directory in the active panel's current location and refresh the panel.
/// Call only when name is non-empty (after trim). Supported on filesystem and inside ZIP archives.
pub fn create_and_refresh(app: &mut AppState, name: &str) {
    let loc = app.get_current_location();
    if let Err(e) = panel_backend::mkdir(&loc, name) {
        eprintln!("Cannot create directory: {}", e);
        return;
    }
    let panel_height = crate::util::compute_panel_height();
    let _ = app.active_panel_mut().refresh_files_restore_selection(
        Some(name),
        None,
        Some(panel_height),
    );
}

/// Handle a key when the mkdir dialog is open. Returns the action to take (MkdirConfirm,
/// MkdirCancel, Suspend, or Continue after updating dialog state).
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    if app.mkdir_dialog.is_none() {
        return None;
    }
    let code = match code {
        KeyCode::Char('\t') => KeyCode::Tab,
        other => other,
    };
    let d = app
        .mkdir_dialog
        .as_mut()
        .expect("mkdir_dialog open when handle_key called");
    match code {
        KeyCode::Tab | KeyCode::Char('\t') => {
            d.focus = (d.focus + 1) % 3;
            return Some(AppAction::Continue);
        }
        KeyCode::BackTab => {
            d.focus = (d.focus + 2) % 3;
            return Some(AppAction::Continue);
        }
        KeyCode::Up => {
            d.focus = (d.focus + 2) % 3;
            return Some(AppAction::Continue);
        }
        KeyCode::Down => {
            d.focus = (d.focus + 1) % 3;
            return Some(AppAction::Continue);
        }
        KeyCode::Enter => {
            return Some(if d.focus == 2 {
                AppAction::MkdirCancel
            } else {
                AppAction::MkdirConfirm
            });
        }
        KeyCode::Esc => return Some(AppAction::MkdirCancel),
        KeyCode::Char(c) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                if c == 'o' {
                    return Some(AppAction::Suspend);
                }
                if c == 'c' {
                    cancel(app);
                    return Some(AppAction::MkdirCancel);
                }
            }
            if d.focus == 0 && c.is_ascii() && !c.is_control() {
                insert_char(app, c);
            }
        }
        KeyCode::Backspace if d.focus == 0 => backspace(app),
        KeyCode::Left if d.focus == 0 => move_left(app),
        KeyCode::Right if d.focus == 0 => move_right(app),
        _ => {}
    }
    Some(AppAction::Continue)
}

/// Draw the "Create a new Directory" dialog: title, prompt, text field with cursor, hint.
pub fn draw(f: &mut Frame, app: &mut AppState) {
    let Some(ref mut d) = app.mkdir_dialog else { return };
    let area = f.area();
    const PAD_H: u16 = 2;
    let max_w = 52u16;
    let w = max_w.min(area.width.saturating_sub(4));
    let h = 9u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };
    let grey_bg = Color::Rgb(60, 60, 60);
    let fill_style = Style::default().bg(grey_bg).fg(Color::White);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Create a new Directory ")
        .style(fill_style.fg(Color::Cyan));
    f.render_widget(block, rect);
    let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
    let content = Rect {
        x: inner.x + PAD_H,
        y: inner.y,
        width: inner.width.saturating_sub(PAD_H * 2),
        height: inner.height,
    };
    let prompt = "Enter directory name:";
    f.render_widget(
        Paragraph::new(prompt).style(fill_style),
        Rect {
            x: content.x,
            y: content.y,
            width: content.width,
            height: 1,
        },
    );
    let input_y = content.y + 2;
    let input_rect = Rect {
        x: content.x,
        y: input_y,
        width: content.width,
        height: 1,
    };
    let input_focused = d.focus == 0;
    let input_bg = if input_focused {
        Color::Rgb(28, 34, 46)
    } else {
        Color::Rgb(38, 44, 56)
    };
    let input_style = Style::default().bg(input_bg).fg(Color::White);
    let input_padded = format!("{:<width$}", d.name, width = content.width as usize);
    f.render_widget(Paragraph::new(input_padded).style(input_style), input_rect);
    if input_focused {
        let cursor_x = content.x
            + (d.name.chars().take(d.cursor).count() as u16).min(content.width.saturating_sub(1));
        if cursor_x < content.x + content.width {
            f.set_cursor_position((cursor_x, input_y));
        }
    }
    const CREATE_W: u16 = 10;
    const CANCEL_W: u16 = 10;
    const BTN_GAP: u16 = 4;
    let total_btns = CREATE_W + CANCEL_W + BTN_GAP;
    let btn_start_x = content.x + content.width.saturating_sub(total_btns) / 2;
    let btn_y = content.y + 5;
    let create_rect = Rect {
        x: btn_start_x,
        y: btn_y,
        width: CREATE_W,
        height: 1,
    };
    let cancel_rect = Rect {
        x: btn_start_x + CREATE_W + BTN_GAP,
        y: btn_y,
        width: CANCEL_W,
        height: 1,
    };
    let create_btn = Line::from(vec![Span::raw("  Create  ")]);
    let cancel_btn = Line::from(vec![Span::raw("  Cancel  ")]);
    let create_style = if d.focus == 1 {
        Style::default().bg(Color::Cyan).fg(Color::Black)
    } else {
        fill_style
    };
    let cancel_style = if d.focus == 2 {
        Style::default().bg(Color::Cyan).fg(Color::Black)
    } else {
        fill_style
    };
    f.render_widget(Paragraph::new(create_btn).style(create_style), create_rect);
    f.render_widget(Paragraph::new(cancel_btn).style(cancel_style), cancel_rect);
}

/// Return (create_button_rect, cancel_button_rect) for mkdir dialog hit-testing.
pub fn mkdir_button_rects(area: ratatui::layout::Rect) -> Option<(Rect, Rect)> {
    const PAD_H: u16 = 2;
    let max_w = 52u16;
    let w = max_w.min(area.width.saturating_sub(4));
    let h = 9u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };
    let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
    let content = Rect {
        x: inner.x + PAD_H,
        y: inner.y,
        width: inner.width.saturating_sub(PAD_H * 2),
        height: inner.height,
    };
    const CREATE_W: u16 = 10;
    const CANCEL_W: u16 = 10;
    const BTN_GAP: u16 = 4;
    let total_btns = CREATE_W + CANCEL_W + BTN_GAP;
    let btn_start_x = content.x + content.width.saturating_sub(total_btns) / 2;
    let btn_y = content.y + 5;
    Some((
        Rect {
            x: btn_start_x,
            y: btn_y,
            width: CREATE_W,
            height: 1,
        },
        Rect {
            x: btn_start_x + CREATE_W + BTN_GAP,
            y: btn_y,
            width: CANCEL_W,
            height: 1,
        },
    ))
}
