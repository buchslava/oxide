//! Ctrl+G "Size info" — shows total size of selected files and folders in the panel's bottom bar.
//! Directories computed recursively. Any key or mouse click returns to file attributes display.

use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app_state::AppState;

/// Message from background size-calculation thread.
#[derive(Debug, Clone)]
pub enum SizeInfoProgress {
    /// One more item processed.
    Progress {
        current: usize,
        total: usize,
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
    /// Calculation complete.
    Done {
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
}

/// State for Ctrl+G "Size info" dialog. Shows progress during calculation, then final result.
#[derive(Debug, Clone)]
pub enum SizeInfoDialogState {
    /// Calculation in progress; show progress bar.
    Calculating {
        current: usize,
        total: usize,
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
    /// Calculation complete; show final size.
    Done {
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
}
use crate::events::AppAction;
use crate::file_ops::FileOperations;
use crate::panel::PanelOperations;
use crate::ui::format_size;

/// Open size info (start background calculation). Ctrl+G. Result shown in panel bottom bar.
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
    });
    app.size_info_pending_rx = Some(rx);
}

/// Close size info and return to file attributes in the bottom bar.
pub fn close(app: &mut AppState) {
    app.size_info_dialog = None;
    app.size_info_pending_rx = None;
}

/// Handle a key when size info is displayed. Any key (except Ctrl+O) closes and returns to attributes.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    if app.size_info_dialog.is_none() {
        return None;
    }
    if let KeyCode::Char(c) = code {
        if modifiers.contains(KeyModifiers::CONTROL) && c == 'o' {
            return Some(AppAction::Suspend);
        }
    }
    close(app);
    Some(AppAction::SizeInfoClose)
}

/// Format the compact line for the panel bottom bar when size info is active. None when not active.
pub fn format_bottom_bar_line(app: &AppState) -> Option<String> {
    let d = app.size_info_dialog.as_ref()?;
    Some(match d {
        SizeInfoDialogState::Calculating {
            current,
            total,
            total_bytes,
            file_count,
            dir_count,
            ..
        } => {
            let size_str = format_size(*total_bytes);
            let count_parts: Vec<String> = [
                (*file_count, "file", "files"),
                (*dir_count, "dir", "dirs"),
            ]
            .into_iter()
            .filter(|(n, ..)| *n > 0)
            .map(|(n, sing, pl)| format!("{} {}", n, if n == 1 { sing } else { pl }))
            .collect();
            if count_parts.is_empty() {
                format!("Calculating {} / {} · {}", current, total, size_str)
            } else {
                format!("Calculating {} / {} · {} ({})", current, total, size_str, count_parts.join(", "))
            }
        }
        SizeInfoDialogState::Done {
            total_bytes,
            file_count,
            dir_count,
        } => {
            let size_str = format_size(*total_bytes);
            let count_parts: Vec<String> = [
                (*file_count, "file", "files"),
                (*dir_count, "dir", "dirs"),
            ]
            .into_iter()
            .filter(|(n, ..)| *n > 0)
            .map(|(n, sing, pl)| format!("{} {}", n, if n == 1 { sing } else { pl }))
            .collect();
            if count_parts.is_empty() {
                format!("Total: {}", size_str)
            } else {
                format!("Total: {} ({})", size_str, count_parts.join(", "))
            }
        }
    })
}
