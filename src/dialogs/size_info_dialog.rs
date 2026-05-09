//! Ctrl+X then S "Size info" — total size of selected files and folders (recursive).
//! While calculating: MC-style centered progress dialog with current path, directory count, total
//! size (KiB), and **[ Abort ]** (Esc or **A**). Background thread honours cancel.
//! When done: same dialog shows totals until any key dismisses (unchanged subshell chord).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use ratatui::layout::{Margin, Rect};

use crate::app::state::AppState;
use crate::browser::panel::PanelOperations;
use crate::core::file_ops::FileOperations;
use crate::core::text_format::format_byte_size;

/// Message from background size-calculation thread.
#[derive(Debug, Clone)]
pub enum SizeInfoProgress {
    /// Live stats during directory scan.
    Progress {
        current_path: String,
        total_bytes: u64,
        directories_scanned: usize,
        files_counted: usize,
    },
    /// Calculation complete.
    Done {
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
}

/// State for Ctrl+X then S "Size info" dialog.
#[derive(Debug, Clone)]
pub enum SizeInfoDialogState {
    Calculating {
        current_path: String,
        total_bytes: u64,
        directories_scanned: usize,
        files_counted: usize,
    },
    Done {
        total_bytes: u64,
        file_count: usize,
        dir_count: usize,
    },
}

const INNER_W: u16 = 72;
const PAD_H: u16 = 2;
const DIALOG_H: u16 = 10;

/// Centered dialog outer rect (must match renderer).
pub fn dialog_rect(area: Rect) -> Rect {
    let w = (INNER_W + 2 + PAD_H * 2).min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(DIALOG_H)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: DIALOG_H,
    }
}

/// `[ Abort ]` button area inside terminal coords (for mouse hit test).
pub fn abort_button_rect(area: Rect) -> Rect {
    let d = dialog_rect(area);
    let inner = d.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let content = Rect {
        x: inner.x + PAD_H,
        y: inner.y,
        width: inner.width.saturating_sub(PAD_H * 2),
        height: inner.height,
    };
    // Must match the `[ Abort ]` line drawn in the renderer (leading space + hotkey A).
    let label = " [ Abort ]";
    let w = label.chars().count() as u16;
    let bx = content.x + (content.width.saturating_sub(w)) / 2;
    let by = content.y + content.height.saturating_sub(1);
    Rect {
        x: bx,
        y: by,
        width: w,
        height: 1,
    }
}

pub fn is_calculating(app: &AppState) -> bool {
    matches!(
        app.size_info_dialog.as_ref(),
        Some(SizeInfoDialogState::Calculating { .. })
    )
}

/// Open size info (start background calculation). Ctrl+X then S.
pub fn open(app: &mut AppState) {
    let cwd = app.get_current_dir().to_string();
    let panel = app.active_panel_ref();
    let (items, ..) = panel.get_names_to_copy_with_restore_neighbors();
    if items.is_empty() {
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    app.size_info_cancel = Some(cancel.clone());
    let top_file_count = items.iter().filter(|(_, d)| !*d).count();
    let top_dir_count = items.iter().filter(|(_, d)| *d).count();
    let items_clone: Vec<(String, bool)> = items.iter().map(|(n, d)| (n.clone(), *d)).collect();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut grand_bytes = 0u64;
        let mut grand_files = 0usize;
        let mut grand_dirs = 0usize;
        for (name, _is_dir) in items_clone.iter() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let path = FileOperations::join_path(&cwd, name);
            let path_ref = std::path::Path::new(&path);
            let base_bytes = grand_bytes;
            let base_files = grand_files;
            let base_dirs = grand_dirs;
            let res = FileOperations::size_of_path_recursive_cancellable(
                path_ref,
                cancel.as_ref(),
                &mut |p, b, f, d| {
                    let _ = tx.send(SizeInfoProgress::Progress {
                        current_path: p.to_string_lossy().into_owned(),
                        total_bytes: base_bytes.saturating_add(b),
                        directories_scanned: base_dirs.saturating_add(d),
                        files_counted: base_files.saturating_add(f),
                    });
                },
            );
            let Some((bytes, f, d)) = res else {
                return;
            };
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            grand_bytes = grand_bytes.saturating_add(bytes);
            grand_files = grand_files.saturating_add(f);
            grand_dirs = grand_dirs.saturating_add(d);
        }
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let _ = tx.send(SizeInfoProgress::Done {
            total_bytes: grand_bytes,
            file_count: top_file_count,
            dir_count: top_dir_count,
        });
    });
    app.size_info_dialog = Some(SizeInfoDialogState::Calculating {
        current_path: String::new(),
        total_bytes: 0,
        directories_scanned: 0,
        files_counted: 0,
    });
    app.size_info_pending_rx = Some(rx);
}

/// Close size info; signals the worker to stop when cancel handle is set.
pub fn close(app: &mut AppState) {
    if let Some(c) = app.size_info_cancel.take() {
        c.store(true, Ordering::Relaxed);
    }
    app.size_info_dialog = None;
    app.size_info_pending_rx = None;
}

/// Format the compact line for the panel bottom bar when size info is active (done phase only).
pub fn format_bottom_bar_line(app: &AppState) -> Option<String> {
    let d = app.size_info_dialog.as_ref()?;
    match d {
        SizeInfoDialogState::Calculating { .. } => None,
        SizeInfoDialogState::Done {
            total_bytes,
            file_count,
            dir_count,
        } => {
            let size_str = format_byte_size(*total_bytes);
            let count_parts: Vec<String> =
                [(*file_count, "file", "files"), (*dir_count, "dir", "dirs")]
                    .into_iter()
                    .filter(|(n, ..)| *n > 0)
                    .map(|(n, sing, pl)| format!("{} {}", n, if n == 1 { sing } else { pl }))
                    .collect();
            if count_parts.is_empty() {
                Some(format!("Total: {}", size_str))
            } else {
                Some(format!(
                    "Total: {} ({})",
                    size_str,
                    count_parts.join(", ")
                ))
            }
        }
    }
}
