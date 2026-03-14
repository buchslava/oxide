//! Copy/move/delete operation state. Used by F5 Copy, F6 Move, F8 Delete and the main loop.

use std::path::PathBuf;

use crate::location::PanelLocation;

/// Copy, Move, or Delete operation (F5 / F6 / F8). Shared flow for progress, overwrite, and error dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Copy,
    Move,
    Delete,
}

/// Progress shown during Copy/Move. Full path of current file, target dir, position (e.g. "3 / 5").
#[derive(Debug, Clone)]
pub struct CopyProgress {
    pub operation: Operation,
    /// Full path of the file/dir being copied/moved (source).
    pub current_path: String,
    /// Destination path (directory or archive path) for display in the progress dialog.
    pub target_path: String,
    pub current: usize,
    pub total: usize,
}

/// Parameters for a copy operation: source dir, target dir, list of (name, is_dir).
/// source_location: when Some, use panel_backend (handles Zip and Fs); when None, use legacy copy_ops with source_dir.
/// target_location: when Some(Zip), copy/move into that archive at its path_inside; when None, use target_fs_path.
/// restore_selection_after/before: after delete/move, try to select this file (after first, else before).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyParams {
    pub source_dir: String,
    pub target_dir: String,
    /// When Some, use panel_backend for copy/move/delete (supports Zip).
    pub source_location: Option<PanelLocation>,
    /// When Some(Zip), copy/move into that archive. When None, target is filesystem and target_fs_path is used.
    pub target_location: Option<PanelLocation>,
    /// When Some, use this as target path for panel_backend copy/move to FS (when opposite panel is Fs).
    pub target_fs_path: Option<PathBuf>,
    pub items: Vec<(String, bool)>,
    pub restore_selection_after: Option<String>,
    pub restore_selection_before: Option<String>,
}

/// In-progress copy/move state: operation, current index, "rewrite all" / "skip all" / "ignore all errors" flags.
#[derive(Debug, Clone)]
pub struct CopyInProgress {
    pub operation: Operation,
    pub params: CopyParams,
    pub current_index: usize,
    pub overwrite_all: bool,
    pub skip_all: bool,
    /// When true, future copy/move errors are skipped without showing the error dialog.
    pub ignore_all_errors: bool,
}

/// Progress shown during Ctrl+A Archive. Same layout as CopyProgress for the overlay (Source, Target, gauge).
#[derive(Debug, Clone)]
pub struct ArchiveProgress {
    /// Full path of the file/dir currently being added (source).
    pub current_path: String,
    /// Full path of the archive being created (target).
    pub target_path: String,
    pub current: usize,
    pub total: usize,
}

/// State when a copy/move error occurred; user picks Skip, Cancel, or Ignore all.
#[derive(Debug, Clone)]
pub struct CopyErrorState {
    pub operation: Operation,
    pub message: String,
}

impl CopyErrorState {
    pub fn new(operation: Operation, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }
}
