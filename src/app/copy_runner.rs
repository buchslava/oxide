//! Copy / move / delete progress: one step per frame, overwrite dialog, zip and legacy paths.

use std::io;
use std::path::Path;
use std::sync::mpsc;

use crate::app::state::{AppState, CopyInProgress, CopyProgress, Operation};
use crate::core::file_ops::FileOperations;
use crate::app::panel_refresh::{refresh_both_panels_full, restore_source_panel_and_refresh};

fn target_fs_dir_for_copy_params(params: &crate::app::state::CopyParams) -> &Path {
    params
        .target_fs_path
        .as_deref()
        .unwrap_or_else(|| Path::new(&params.target_dir))
}

fn source_item_display_path(
    params: &crate::app::state::CopyParams,
    name: &str,
) -> String {
    params
        .source_location
        .as_ref()
        .map(|loc| crate::core::panel_backend::join_path_display(loc, name))
        .unwrap_or_else(|| {
            FileOperations::join_path(&params.source_dir, name)
                .to_string_lossy()
                .to_string()
        })
}

fn target_display_for_copy_progress(
    operation: Operation,
    params: &crate::app::state::CopyParams,
) -> String {
    match operation {
        Operation::Copy | Operation::Move => params
            .target_location
            .as_ref()
            .map(|loc| loc.display_string())
            .unwrap_or_else(|| params.target_dir.trim_end_matches('/').to_string()),
        Operation::Delete => String::new(),
    }
}

fn apply_panel_location_copy_step(
    c: &CopyInProgress,
    source_loc: &crate::core::location::PanelLocation,
    name: &str,
    is_dir: bool,
    current_path: &str,
) -> (bool, Option<String>, Option<String>) {
    match c.params.target_location.as_ref() {
        Some(crate::core::location::PanelLocation::Zip {
            archive,
            path_inside,
        }) => run_copy_step_into_archive(
            source_loc,
            name,
            is_dir,
            current_path,
            archive,
            path_inside,
            c.operation,
            c.ignore_all_errors,
        ),
        _ => {
            let target_dir = target_fs_dir_for_copy_params(&c.params);
            run_copy_step_backend(
                source_loc,
                name,
                is_dir,
                current_path,
                target_dir,
                c.operation,
                c.overwrite_all,
                c.skip_all,
                c.ignore_all_errors,
            )
        }
    }
}

fn poll_pending_directory_delete(
    app: &mut AppState,
    c: &mut CopyInProgress,
    current_path: &str,
) -> bool {
    let Some(rx) = app.delete_pending_rx.take() else {
        return false;
    };
    match rx.try_recv() {
        Ok(Ok(())) => {
            c.current_index += 1;
            true
        }
        Ok(Err(e)) => {
            if c.ignore_all_errors {
                c.current_index += 1;
            } else {
                app.copy_error_dialog = Some(crate::app::state::CopyErrorState::new(
                    c.operation,
                    format!("{}: {}", current_path, e),
                ));
                app.copy_error_focus = 0;
            }
            true
        }
        Err(mpsc::TryRecvError::Empty) => {
            app.delete_pending_rx = Some(rx);
            true
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            if !c.ignore_all_errors {
                app.copy_error_dialog = Some(crate::app::state::CopyErrorState::new(
                    c.operation,
                    format!("{}: delete failed", current_path),
                ));
                app.copy_error_focus = 0;
            } else {
                c.current_index += 1;
            }
            true
        }
    }
}

fn spawn_background_directory_delete(
    app: &mut AppState,
    source_dir: String,
    entry_name: String,
) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::core::copy_ops::delete_item(
            &source_dir,
            &entry_name,
            true,
        ));
    });
    app.delete_pending_rx = Some(rx);
}

fn run_copy_step_legacy_copy_move(
    app: &mut AppState,
    c: &mut CopyInProgress,
    name: &str,
    is_dir: bool,
) {
    let target_path = FileOperations::join_path(&c.params.target_dir, name);
    if target_path.exists() && !c.overwrite_all && !c.skip_all {
        app.copy_overwrite_dialog = Some(name.to_string());
        app.copy_overwrite_focus = 0;
        return;
    }
    let do_op = |op: Operation| {
        if op == Operation::Move {
            crate::core::copy_ops::move_item(
                &c.params.source_dir,
                &c.params.target_dir,
                name,
                is_dir,
            )
        } else {
            crate::core::copy_ops::copy_item(
                &c.params.source_dir,
                &c.params.target_dir,
                name,
                is_dir,
            )
        }
    };
    if target_path.exists() && c.skip_all {
        c.current_index += 1;
        return;
    }
    if target_path.exists() && c.overwrite_all {
        let result = do_op(c.operation);
        if apply_copy_item_io_outcome(app, c, name, result) {
            c.current_index += 1;
        }
        return;
    }
    if !target_path.exists() {
        let result = do_op(c.operation);
        if apply_copy_item_io_outcome(app, c, name, result) {
            c.current_index += 1;
        }
    }
}

fn run_copy_step_legacy_delete(
    app: &mut AppState,
    c: &mut CopyInProgress,
    name: &str,
    is_dir: bool,
    current_path: &str,
) {
    if poll_pending_directory_delete(app, c, current_path) {
        return;
    }
    if is_dir {
        spawn_background_directory_delete(app, c.params.source_dir.clone(), name.to_string());
        return;
    }
    match crate::core::copy_ops::delete_item(&c.params.source_dir, name, is_dir) {
        Ok(()) => c.current_index += 1,
        Err(e) => {
            if c.ignore_all_errors {
                c.current_index += 1;
            } else {
                app.copy_error_dialog = Some(crate::app::state::CopyErrorState::new(
                    c.operation,
                    format!("{}: {}", current_path, e),
                ));
                app.copy_error_focus = 0;
            }
        }
    }
}

/// Start a copy/move operation and clear overwrite/error dialogs.
pub(crate) fn start_copy_operation(
    app: &mut AppState,
    operation: Operation,
    params: crate::app::state::CopyParams,
) {
    let total = params.items.len();
    let initial_path = params
        .items
        .first()
        .map(|(name, _)| {
            if let Some(loc) = params.source_location.as_ref() {
                crate::core::panel_backend::join_path_display(loc, name)
            } else {
                FileOperations::join_path(&params.source_dir, name)
                    .to_string_lossy()
                    .to_string()
            }
        })
        .unwrap_or_default();

    let target_path = match operation {
        Operation::Copy | Operation::Move => params
            .target_location
            .as_ref()
            .map(|loc| loc.display_string())
            .unwrap_or_else(|| params.target_dir.trim_end_matches('/').to_string()),
        Operation::Delete => String::new(),
    };

    app.copy_in_progress = Some(CopyInProgress {
        operation,
        params,
        current_index: 0,
        overwrite_all: false,
        skip_all: false,
        ignore_all_errors: false,
    });
    app.copy_progress = Some(CopyProgress {
        operation,
        current_path: initial_path,
        target_path,
        current: 1,
        total,
    });
    app.copy_overwrite_dialog = None;
    app.copy_error_dialog = None;
}

/// One step of copy/move/delete when source is PanelLocation (Fs or Zip). Returns (advance, overwrite_name, error_message).
fn run_copy_step_backend(
    source_loc: &crate::core::location::PanelLocation,
    name: &str,
    is_dir: bool,
    current_path: &str,
    target_dir: &Path,
    operation: Operation,
    overwrite_all: bool,
    skip_all: bool,
    ignore_all_errors: bool,
) -> (bool, Option<String>, Option<String>) {
    let target_path = target_dir.join(name.trim_end_matches('/'));

    if operation == Operation::Delete {
        let items = &[(name.to_string(), is_dir)];
        return match crate::core::panel_backend::delete_items(source_loc, items) {
            Ok(()) => (true, None, None),
            Err(e) => (
                ignore_all_errors,
                None,
                Some(format!("{}: {}", current_path, e)),
            ),
        };
    }

    if target_path.exists() && !overwrite_all && !skip_all {
        return (false, Some(name.to_string()), None);
    }
    if target_path.exists() && skip_all {
        return (true, None, None);
    }

    let items = &[(name.to_string(), is_dir)];
    let result = if operation == Operation::Move {
        crate::core::panel_backend::move_items_to_fs(source_loc, items, target_dir)
    } else {
        crate::core::panel_backend::copy_items_to_fs(source_loc, items, target_dir)
    };

    match result {
        Ok(()) => (true, None, None),
        Err(e) => (
            ignore_all_errors,
            None,
            Some(format!("{} -> {}: {}", current_path, name, e)),
        ),
    }
}

/// One step of copy/move when target is inside a ZIP. No overwrite dialog; existing entries are overwritten.
fn run_copy_step_into_archive(
    source_loc: &crate::core::location::PanelLocation,
    name: &str,
    is_dir: bool,
    current_path: &str,
    archive_path: &Path,
    path_inside: &str,
    operation: Operation,
    ignore_all_errors: bool,
) -> (bool, Option<String>, Option<String>) {
    let items = &[(name.to_string(), is_dir)];
    let result = if operation == Operation::Move {
        crate::core::panel_backend::move_items_into_archive(
            source_loc,
            items,
            archive_path,
            path_inside,
        )
    } else {
        crate::core::panel_backend::copy_items_into_archive(
            source_loc,
            items,
            archive_path,
            path_inside,
        )
    };
    match result {
        Ok(()) => (true, None, None),
        Err(e) => (
            ignore_all_errors,
            None,
            Some(format!("{} -> (archive): {}", current_path, e)),
        ),
    }
}

/// Advance the in-progress copy/move by one item. If target exists and no overwrite_all/skip_all, shows overwrite dialog.
pub(crate) fn run_copy_step(app: &mut AppState) {
    let Some(active_copy) = app.copy_in_progress.as_ref() else {
        return;
    };
    let total = active_copy.params.items.len();
    if active_copy.current_index >= total {
        let source_dir = active_copy.params.source_dir.clone();
        let restore_after = active_copy.params.restore_selection_after.clone();
        let restore_before = active_copy.params.restore_selection_before.clone();
        app.copy_in_progress = None;
        app.copy_progress = None;
        app.delete_pending_rx = None;
        app.source_panel_restore = Some((source_dir, restore_after, restore_before));
        return;
    }

    let (name, is_dir, current_path, use_panel_backend) = {
        let Some(c) = app.copy_in_progress.as_ref() else {
            return;
        };
        let (name, is_dir) = &c.params.items[c.current_index];
        (
            name.clone(),
            *is_dir,
            source_item_display_path(&c.params, name),
            c.params.source_location.is_some(),
        )
    };

    {
        let Some(c) = app.copy_in_progress.as_ref() else {
            return;
        };
        app.copy_progress = Some(CopyProgress {
            operation: c.operation,
            current_path: current_path.clone(),
            target_path: target_display_for_copy_progress(c.operation, &c.params),
            current: c.current_index + 1,
            total,
        });
    }

    if use_panel_backend {
        let (advance, overwrite_name, error_msg) = {
            let Some(c) = app.copy_in_progress.as_ref() else {
                return;
            };
            let Some(source_loc) = c.params.source_location.as_ref() else {
                return;
            };
            apply_panel_location_copy_step(c, source_loc, &name, is_dir, &current_path)
        };
        let Some(c) = app.copy_in_progress.as_mut() else {
            return;
        };
        if let Some(n) = overwrite_name {
            app.copy_overwrite_dialog = Some(n);
            app.copy_overwrite_focus = 0;
            return;
        }
        if let Some(msg) = error_msg {
            if !advance {
                app.copy_error_dialog = Some(crate::app::state::CopyErrorState::new(c.operation, msg));
                app.copy_error_focus = 0;
            }
        }
        if advance {
            c.current_index += 1;
        }
        return;
    }

    let mut c = match app.copy_in_progress.take() {
        Some(c) => c,
        None => return,
    };
    if c.operation == Operation::Delete {
        run_copy_step_legacy_delete(app, &mut c, &name, is_dir, &current_path);
    } else {
        run_copy_step_legacy_copy_move(app, &mut c, &name, is_dir);
    }
    app.copy_in_progress = Some(c);
}

fn perform_copy_overwrite_item_io(
    c: &mut CopyInProgress,
    name: &str,
    is_dir: bool,
) -> io::Result<()> {
    if let Some(ref loc) = c.params.source_location {
        let items = &[(name.to_string(), is_dir)];
        match c.params.target_location.as_ref() {
            Some(crate::core::location::PanelLocation::Zip {
                archive,
                path_inside,
            }) => {
                if c.operation == Operation::Move {
                    crate::core::panel_backend::move_items_into_archive(
                        loc,
                        items,
                        archive,
                        path_inside,
                    )
                } else {
                    crate::core::panel_backend::copy_items_into_archive(
                        loc,
                        items,
                        archive,
                        path_inside,
                    )
                }
            }
            _ => {
                let target = target_fs_dir_for_copy_params(&c.params);
                if c.operation == Operation::Move {
                    crate::core::panel_backend::move_items_to_fs(loc, items, target)
                } else {
                    crate::core::panel_backend::copy_items_to_fs(loc, items, target)
                }
            }
        }
    } else if c.operation == Operation::Move {
        crate::core::copy_ops::move_item(&c.params.source_dir, &c.params.target_dir, name, is_dir)
    } else {
        crate::core::copy_ops::copy_item(&c.params.source_dir, &c.params.target_dir, name, is_dir)
    }
}

fn apply_copy_item_io_outcome(
    app: &mut AppState,
    progress: &CopyInProgress,
    name: &str,
    result: io::Result<()>,
) -> bool {
    match result {
        Ok(()) => true,
        Err(e) => {
            if progress.ignore_all_errors {
                true
            } else {
                app.copy_error_dialog = Some(crate::app::state::CopyErrorState::new(
                    progress.operation,
                    format!("{} -> {}: {}", progress.params.source_dir, name, e),
                ));
                app.copy_error_focus = 0;
                false
            }
        }
    }
}

fn finish_copy_if_no_items_left(app: &mut AppState) {
    let should_finish = match app.copy_in_progress.as_ref() {
        Some(c) => c.current_index >= c.params.items.len(),
        None => return,
    };
    if !should_finish {
        return;
    }
    let (source_dir, restore_after, restore_before) = {
        let Some(c) = app.copy_in_progress.as_ref() else {
            return;
        };
        (
            c.params.source_dir.clone(),
            c.params.restore_selection_after.clone(),
            c.params.restore_selection_before.clone(),
        )
    };
    app.copy_in_progress = None;
    app.copy_progress = None;
    app.delete_pending_rx = None;
    restore_source_panel_and_refresh(
        app,
        &source_dir,
        restore_after.as_deref(),
        restore_before.as_deref(),
    );
}

pub(crate) fn handle_copy_overwrite_choice(
    app: &mut AppState,
    n: usize,
) {
    if n == 5 {
        app.copy_in_progress = None;
        app.copy_progress = None;
        app.delete_pending_rx = None;
        refresh_both_panels_full(app);
    } else if let Some(mut c) = app.copy_in_progress.take() {
        let idx = c.current_index;
        if idx < c.params.items.len() {
            let (name, is_dir) = c.params.items[idx].clone();
            let mut advance = false;
            match n {
                1 => {
                    let result = perform_copy_overwrite_item_io(&mut c, &name, is_dir);
                    advance = apply_copy_item_io_outcome(app, &c, &name, result);
                }
                2 => {
                    c.overwrite_all = true;
                    let result = perform_copy_overwrite_item_io(&mut c, &name, is_dir);
                    advance = apply_copy_item_io_outcome(app, &c, &name, result);
                }
                3 => advance = true,
                4 => {
                    c.skip_all = true;
                    advance = true;
                }
                _ => {}
            }
            if advance {
                c.current_index += 1;
            }
        }
        app.copy_in_progress = Some(c);
    }
    app.copy_overwrite_dialog = None;
    finish_copy_if_no_items_left(app);
}
