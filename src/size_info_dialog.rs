//! F9 "Size info" dialog. Shows total size of selected files and folders (directories computed recursively).
//! Opens immediately with progress bar during calculation. Esc closes the dialog.

use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
    Frame,
};

use crate::app_state::{AppState, SizeInfoDialogState, SizeInfoProgress};
use crate::events::AppAction;
use crate::file_ops::FileOperations;
use crate::panel::PanelOperations;
use crate::ui::{format_size, truncate_str_ellipsis};

/// Open the dialog immediately and start background size calculation. F9.
pub fn open(app: &mut AppState) {
    let cwd = app.get_current_dir().to_string();
    let panel = app.active_panel_ref();
    let (items, ..) = panel.get_names_to_copy_with_restore_neighbors();
    if items.is_empty() {
        return;
    }
    let total = items.len();
    let (tx, rx) = mpsc::channel();
    let items_clone: Vec<(String, bool)> = items.iter().map(|(n, d)| (n.clone(), *d)).collect();
    std::thread::spawn(move || {
        let mut total_bytes = 0u64;
        let mut file_count = 0usize;
        let mut dir_count = 0usize;
        for (i, (name, is_dir)) in items_clone.iter().enumerate() {
            let path = FileOperations::join_path(&cwd, name);
            if *is_dir {
                dir_count += 1;
                total_bytes = total_bytes.saturating_add(FileOperations::size_of_path_recursive(&path));
            } else {
                file_count += 1;
                total_bytes = total_bytes.saturating_add(FileOperations::size_of_path_recursive(&path));
            }
            let current = i + 1;
            let _ = tx.send(SizeInfoProgress::Progress {
                current,
                total,
                total_bytes,
                file_count,
                dir_count,
                current_name: name.clone(),
            });
        }
        let _ = tx.send(SizeInfoProgress::Done {
            total_bytes,
            file_count,
            dir_count,
        });
    });
    app.size_info_dialog = Some(SizeInfoDialogState::Calculating {
        current: 0,
        total,
        total_bytes: 0,
        file_count: 0,
        dir_count: 0,
        current_name: items.first().map(|(n, _)| n.clone()).unwrap_or_default(),
    });
    app.size_info_pending_rx = Some(rx);
}

/// Close the dialog.
pub fn close(app: &mut AppState) {
    app.size_info_dialog = None;
    app.size_info_pending_rx = None;
}

/// Handle a key when the size info dialog is open. Returns the action (SizeInfoClose or Continue).
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    if app.size_info_dialog.is_none() {
        return None;
    }
    match code {
        KeyCode::Esc => {
            close(app);
            return Some(AppAction::SizeInfoClose);
        }
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) && c == 'o' => {
            return Some(AppAction::Suspend);
        }
        _ => {}
    }
    Some(AppAction::Continue)
}

/// Draw the "Size info" dialog: progress bar while calculating, then total size and counts.
pub fn draw(f: &mut Frame, app: &mut AppState) {
    let Some(ref d) = app.size_info_dialog else { return };
    let area = f.area();
    let inner_width = 60usize;
    const PAD_H: u16 = 2;
    let w = (inner_width as u16 + 2 + PAD_H * 2).min(area.width.saturating_sub(4));
    let h = 10u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };
    let grey_bg = Color::Rgb(60, 60, 60);
    let fill_style = Style::default().bg(grey_bg).fg(Color::White);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Size of selected ")
        .style(fill_style.fg(Color::Cyan));
    f.render_widget(block, rect);
    let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
    let content = Rect {
        x: inner.x + PAD_H,
        y: inner.y,
        width: inner.width.saturating_sub(PAD_H * 2),
        height: inner.height,
    };
    let max_path_w = content.width as usize;

    match d {
        SizeInfoDialogState::Calculating {
            current,
            total,
            total_bytes,
            file_count,
            dir_count,
            current_name,
        } => {
            let src_label = Paragraph::new("Calculating…")
                .style(fill_style.fg(Color::Yellow))
                .alignment(Alignment::Center);
            f.render_widget(src_label, Rect {
                x: content.x,
                y: content.y,
                width: content.width,
                height: 1,
            });
            let path_display = truncate_str_ellipsis(current_name, max_path_w.max(10));
            f.render_widget(
                Paragraph::new(path_display).style(fill_style).alignment(Alignment::Center),
                Rect {
                    x: content.x,
                    y: content.y + 1,
                    width: content.width,
                    height: 1,
                },
            );
            let ratio = if *total > 0 {
                (*current as f64) / (*total as f64).max(1.0)
            } else {
                0.0
            };
            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Cyan))
                .ratio(ratio)
                .label(format!("{} / {}", current, total));
            f.render_widget(gauge, Rect {
                x: content.x,
                y: content.y + 2,
                width: content.width,
                height: 1,
            });
            let size_so_far = format_size(*total_bytes);
            let count_parts: Vec<String> = [
                (*file_count, "file", "files"),
                (*dir_count, "dir", "dirs"),
            ]
            .into_iter()
            .filter(|(n, ..)| *n > 0)
            .map(|(n, sing, pl)| format!("{} {}", n, if n == 1 { sing } else { pl }))
            .collect();
            let status = if count_parts.is_empty() {
                size_so_far
            } else {
                format!("{} ({})", size_so_far, count_parts.join(", "))
            };
            f.render_widget(
                Paragraph::new(status).style(fill_style).alignment(Alignment::Center),
                Rect {
                    x: content.x,
                    y: content.y + 3,
                    width: content.width,
                    height: 1,
                },
            );
        }
        SizeInfoDialogState::Done {
            total_bytes,
            file_count,
            dir_count,
        } => {
            let size_str = format_size(*total_bytes);
            let total_line = format!("Total: {}", size_str);
            f.render_widget(
                Paragraph::new(total_line.as_str())
                    .style(fill_style)
                    .alignment(Alignment::Center),
                Rect {
                    x: content.x,
                    y: content.y + 1,
                    width: content.width,
                    height: 1,
                },
            );
            let count_parts: Vec<String> = [
                (*file_count, "file", "files"),
                (*dir_count, "directory", "directories"),
            ]
            .into_iter()
            .filter(|(n, ..)| *n > 0)
            .map(|(n, sing, pl)| format!("{} {}", n, if n == 1 { sing } else { pl }))
            .collect();
            let count_line = count_parts.join(", ");
            if !count_line.is_empty() {
                f.render_widget(
                    Paragraph::new(count_line.as_str())
                        .style(fill_style)
                        .alignment(Alignment::Center),
                    Rect {
                        x: content.x,
                        y: content.y + 2,
                        width: content.width,
                        height: 1,
                    },
                );
            }
        }
    }

    let hint_rect = Rect {
        x: content.x,
        y: content.y + content.height.saturating_sub(2),
        width: content.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new("Esc or click to close")
            .style(fill_style.fg(Color::DarkGray))
            .alignment(Alignment::Center),
        hint_rect,
    );
}
