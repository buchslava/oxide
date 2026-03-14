//! Ctrl+A "Archive" dialog (MC-style). Single text field for the archive file name;
//! selected items are zipped into it; originals are kept. Enter = create (if non-empty), Esc = cancel.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app_state::{AppState, ArchiveMessage, ArchiveProgress};
use crate::events::AppAction;
use crate::file_ops::FileOperations;
use crate::location::PanelLocation;

/// State for Ctrl+A "Archive" dialog. Single text field for the archive file name (e.g. archive.zip).
/// focus: 0 = textarea, 1 = Create, 2 = Cancel.
#[derive(Debug, Clone)]
pub struct ArchiveDialogState {
    pub name: String,
    pub cursor: usize,
    pub focus: usize,
}

/// Open the dialog with an empty archive name. Call only when at least one item is selected and location is Fs.
pub fn open(app: &mut AppState) {
    let default_name = String::new();
    let cursor = default_name.len();
    app.archive_dialog = Some(ArchiveDialogState {
        name: default_name,
        cursor,
        focus: 0,
    });
}

/// Close the dialog without creating. Called on Esc or Ctrl+C.
pub fn cancel(app: &mut AppState) {
    app.archive_dialog = None;
}

/// Take the entered name and close the dialog. Returns the name (may be empty).
pub fn confirm(app: &mut AppState) -> Option<String> {
    app.archive_dialog.take().map(|d| d.name)
}

/// Create the archive in the active panel's current location and refresh the panel (sync, no progress).
/// Kept for compatibility; normal flow uses start_archive_background.
#[allow(dead_code)]
pub fn create_and_refresh(
    app: &mut AppState,
    name: &str,
    items: &[(String, bool)],
) {
    let loc = app.get_current_location();
    if let Err(e) = crate::panel_backend::create_archive(&loc, items, name) {
        eprintln!("Cannot create archive: {}", e);
        return;
    }
    let panel_height = crate::util::compute_panel_height();
    let _ = app.active_panel_mut().refresh_files_restore_selection(
        Some(name),
        None,
        Some(panel_height),
    );
}

/// Start creating the archive in a background thread; show progress overlay. Call after closing the dialog.
pub fn start_archive_background(app: &mut AppState, name: &str, items: &[(String, bool)]) {
    let loc = app.get_current_location();
    let PanelLocation::Fs(base_dir) = &loc else {
        eprintln!("Archive only supported on filesystem");
        return;
    };
    let items: Vec<(String, bool)> = items.to_vec();
    let name = name.to_string();
    let target_path = FileOperations::join_path(base_dir, &name).to_string_lossy().to_string();

    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));

    let total = items.len();
    let first_path = items.first().map(|(n, _)| FileOperations::join_path(base_dir, n).to_string_lossy().to_string()).unwrap_or_default();
    app.archive_progress = Some(ArchiveProgress {
        current_path: first_path,
        target_path: target_path.clone(),
        current: 0,
        total,
    });
    app.archive_pending_rx = Some(rx);
    app.archive_cancel = Some(Arc::clone(&cancel));

    let loc_clone = loc.clone();
    let _ = std::thread::spawn(move || {
        let mut progress = |current: usize, total: usize, current_path: &str| {
            let _ = tx.send(ArchiveMessage::Progress(ArchiveProgress {
                current_path: current_path.to_string(),
                target_path: target_path.clone(),
                current,
                total,
            }));
        };
        let result = crate::panel_backend::create_archive_with_progress(
            &loc_clone,
            &items,
            &name,
            &mut progress,
            Some(&cancel),
        );
        let name_for_selection = result.as_ref().ok().map(|_| name.clone());
        let _ = tx.send(ArchiveMessage::Done(result, name_for_selection));
    });
}

/// Handle a key when the archive dialog is open. Returns the action to take (ArchiveConfirm,
/// ArchiveCancel, Suspend, or Continue after updating dialog state).
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    if app.archive_dialog.is_none() {
        return None;
    }
    let code = match code {
        KeyCode::Char('\t') => KeyCode::Tab,
        other => other,
    };
    let d = app
        .archive_dialog
        .as_mut()
        .expect("archive_dialog open when handle_key called");
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
                AppAction::ArchiveCancel
            } else {
                AppAction::ArchiveConfirm
            });
        }
        KeyCode::Esc => return Some(AppAction::ArchiveCancel),
        KeyCode::Char(c) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                if c == 'o' {
                    return Some(AppAction::Suspend);
                }
                if c == 'c' {
                    cancel(app);
                    return Some(AppAction::ArchiveCancel);
                }
            }
            if d.focus == 0 && c.is_ascii() && !c.is_control() {
                let at = d.cursor.min(d.name.len());
                d.name.insert(at, c);
                d.cursor = at + 1;
            }
        }
        KeyCode::Backspace if d.focus == 0 => {
            if d.cursor > 0 && d.cursor <= d.name.len() {
                d.name.remove(d.cursor - 1);
                d.cursor -= 1;
            }
        }
        KeyCode::Left if d.focus == 0 => {
            if d.cursor > 0 {
                d.cursor -= 1;
            }
        }
        KeyCode::Right if d.focus == 0 => {
            if d.cursor < d.name.len() {
                d.cursor += 1;
            }
        }
        _ => {}
    }
    Some(AppAction::Continue)
}

/// Draw the "Archive" dialog: title, prompt, text field with cursor, hint.
pub fn draw(f: &mut Frame, app: &mut AppState) {
    let Some(ref mut d) = app.archive_dialog else { return };
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
        .title(" Archive ")
        .style(fill_style.fg(Color::Cyan));
    f.render_widget(block, rect);
    let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
    let content = Rect {
        x: inner.x + PAD_H,
        y: inner.y,
        width: inner.width.saturating_sub(PAD_H * 2),
        height: inner.height,
    };
    let prompt = "Enter archive file name:";
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

/// Return (create_button_rect, cancel_button_rect) for archive dialog hit-testing.
pub fn archive_button_rects(area: ratatui::layout::Rect) -> Option<(Rect, Rect)> {
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
