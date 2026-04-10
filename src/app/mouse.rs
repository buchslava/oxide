//! Mouse hit-testing and dispatch for the panel UI. Modal layer order matches [`crate::app::events::EventHandler::dispatch_event`].
//! Keyboard paths for the same dialogs live in `events.rs` — behavior is mirrored, not shared, to avoid coupling.

use crate::app::events::{AppAction, CopyErrorChoice, DeleteConfirmChoice, EditorConfirmChoice};
use crate::app::state::{AppState, CopyParams, Focus, Operation};
use crate::browser::editor::{editor_confirm_option_rects, save_changes_confirm_rect};
use crate::browser::panel::{PanelOperations, ViewMode};
use crate::core::location::PanelLocation;
use crate::core::panel_backend::{supports_edit, supports_mkdir};
use crate::dialogs::{
    find_dialog, help_dialog, panel_overlay, pattern_select_dialog, rename_attr, settings_dialog,
};
use crate::ui::dialog_layout;
use crate::ui::menu_bar_key;
use crate::ui::Renderer;
use crate::util::compute_panel_height;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use crossterm::terminal::size;
use ratatui::layout::Rect;
use std::io;

/// First content row (below title) for overwrite / copy-error numbered option lists in [`Renderer`].
const NUMBERED_OPTIONS_FIRST_ROW: u16 = 2;

/// When false, mouse must not scroll panels, click the file list, or use the bottom menu bar
/// (dialogs and progress overlays are modal).
fn panels_mouse_enabled(app: &AppState) -> bool {
    app.diff_viewer_screen.is_none()
        && app.viewer_screen.is_none()
        && app.editor_screen.is_none()
        && !app.editor_confirm_pending
        && app.copy_overwrite_dialog.is_none()
        && app.copy_error_dialog.is_none()
        && app.operation_confirm_pending.is_none()
        && app.mkdir_dialog.is_none()
        && app.pattern_select_dialog.is_none()
        && app.archive_dialog.is_none()
        && app.new_file_dialog.is_none()
        && app.new_file_error.is_none()
        && app.rename_attr_dialog.is_none()
        && app.settings_dialog.is_none()
        && app.help_dialog.is_none()
        && app.find_dialog.is_none()
        && app.left_panel_settings_overlay.is_none()
        && app.right_panel_settings_overlay.is_none()
        && app.copy_in_progress.is_none()
        && app.archive_progress.is_none()
}

/// Which of two primary/secondary dialog buttons was hit (Create/Cancel, Apply/Cancel, Yes/No).
#[derive(Clone, Copy)]
enum TwinButton {
    Primary,
    Secondary,
}

fn hit_test_twin_buttons(
    col: u16,
    row: u16,
    primary: Rect,
    secondary: Rect,
) -> Option<TwinButton> {
    if dialog_layout::pointer_in_dialog(col, row, primary) {
        Some(TwinButton::Primary)
    } else if dialog_layout::pointer_in_dialog(col, row, secondary) {
        Some(TwinButton::Secondary)
    } else {
        None
    }
}

/// Row index 0..count inside numbered option rows starting at `content.y + NUMBERED_OPTIONS_FIRST_ROW`.
fn numbered_option_index(
    col: u16,
    row: u16,
    content: Rect,
    count: usize,
) -> Option<usize> {
    if col < content.x
        || col >= content.x + content.width
        || row < content.y + NUMBERED_OPTIONS_FIRST_ROW
    {
        return None;
    }
    let opt_row = (row - content.y - NUMBERED_OPTIONS_FIRST_ROW) as usize;
    (opt_row < count).then_some(opt_row)
}

/// Left click outside `modal_rect` → `outside_action`. While `active`, always returns `Some` to consume the event.
fn try_modal_left_click_outside_rect(
    active: bool,
    mouse_event: &MouseEvent,
    modal_rect: Rect,
    outside_action: AppAction,
) -> Option<AppAction> {
    if !active {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (col, row) = (mouse_event.column, mouse_event.row);
        if !dialog_layout::pointer_in_dialog(col, row, modal_rect) {
            return Some(outside_action);
        }
    }
    Some(AppAction::Continue)
}

/// Panel settings overlay: left click outside the overlay box closes.
fn try_panel_overlay_outside_click(
    overlay_open: bool,
    mouse_event: &MouseEvent,
    panel_bounds: Option<Rect>,
    outside_action: AppAction,
) -> Option<AppAction> {
    if !overlay_open {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (col, row) = (mouse_event.column, mouse_event.row);
        if let Some(bounds) = panel_bounds {
            let rect = panel_overlay::overlay_dialog_rect(bounds);
            if !dialog_layout::pointer_in_dialog(col, row, rect) {
                return Some(outside_action);
            }
        }
    }
    Some(AppAction::Continue)
}

/// Modal with a bounding rect and two footer buttons (outside / primary / secondary).
fn try_mouse_modal_rect_twin_buttons(
    active: bool,
    mouse_event: &MouseEvent,
    modal_rect: Rect,
    buttons: Option<(Rect, Rect)>,
    outside: AppAction,
    primary: AppAction,
    secondary: AppAction,
) -> Option<AppAction> {
    if !active {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (col, row) = (mouse_event.column, mouse_event.row);
        if !dialog_layout::pointer_in_dialog(col, row, modal_rect) {
            return Some(outside);
        }
        if let Some((p, s)) = buttons {
            match hit_test_twin_buttons(col, row, p, s) {
                Some(TwinButton::Primary) => return Some(primary),
                Some(TwinButton::Secondary) => return Some(secondary),
                None => {}
            }
        }
    }
    Some(AppAction::Continue)
}

/// Mkdir / archive / new file: shared single-input dialog hit-test ([`dialog_layout::hit_test_single_input_dialog`]).
fn try_mouse_single_input_dialog(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
    active: bool,
    get_text: impl FnOnce(&AppState) -> String,
    on_confirm: impl FnOnce(String) -> AppAction,
    on_dismiss: AppAction,
) -> Option<AppAction> {
    if !active {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (col, row) = (mouse_event.column, mouse_event.row);
        use dialog_layout::SingleInputDialogHit;
        match dialog_layout::hit_test_single_input_dialog(col, row, area) {
            SingleInputDialogHit::Outside | SingleInputDialogHit::Secondary => {
                return Some(on_dismiss);
            }
            SingleInputDialogHit::Primary => return Some(on_confirm(get_text(app))),
            SingleInputDialogHit::InsideBody => {}
        }
    }
    Some(AppAction::Continue)
}

/// Returns Ok(Some(action)) when an action (e.g. RunCommand) should be handled by the main loop.
pub(crate) fn handle_mouse_event(
    app: &mut AppState,
    mouse_event: MouseEvent,
) -> io::Result<Option<AppAction>> {
    app.last_mouse_position = Some((mouse_event.column, mouse_event.row));

    let (term_w, term_h) = size().unwrap_or((80, 24));
    let area = Rect {
        x: 0,
        y: 0,
        width: term_w,
        height: term_h,
    };

    if let Some(out) = try_mouse_editor_confirm(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_copy_overwrite(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_copy_error(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if app.copy_progress.is_some() || app.archive_progress.is_some() {
        return Ok(Some(AppAction::Continue));
    }
    if let Some(out) = try_mouse_pattern_select(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_mkdir(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_archive(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_new_file_error(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_new_file(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_help(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_settings(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_size_info(app, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_find(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_panel_overlay_outside_click(
        app.left_panel_settings_overlay.is_some(),
        &mouse_event,
        app.left_panel_rect,
        AppAction::CloseLeftPanelSettings,
    ) {
        return Ok(Some(out));
    }
    if let Some(out) = try_panel_overlay_outside_click(
        app.right_panel_settings_overlay.is_some(),
        &mouse_event,
        app.right_panel_rect,
        AppAction::CloseRightPanelSettings,
    ) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_rename_attr(app, area, &mouse_event) {
        return Ok(Some(out));
    }
    if let Some(out) = try_mouse_operation_confirm(app, area, &mouse_event) {
        return Ok(Some(out));
    }

    let panel_height = compute_panel_height();
    let panels_mouse = panels_mouse_enabled(app);

    match mouse_event.kind {
        MouseEventKind::ScrollUp => {
            if panels_mouse {
                app.active_panel_mut().move_up(panel_height);
            }
        }
        MouseEventKind::ScrollDown => {
            if panels_mouse {
                app.active_panel_mut().move_down(panel_height);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if mouse_event.row == term_h.saturating_sub(1) {
                if panels_mouse {
                    if let Some(action) = hit_test_menu_bar(mouse_event.column, term_w, app) {
                        return Ok(Some(action));
                    }
                }
            }
            if panels_mouse {
                if let Some((panel_index, file_index)) =
                    hit_test_panel(mouse_event.column, mouse_event.row, term_w, term_h, app)
                {
                    app.focus_panel();
                    app.set_active_panel(panel_index);
                    let panel = if panel_index == 0 {
                        app.left_panel_mut()
                    } else {
                        app.right_panel_mut()
                    };
                    panel.set_selection(file_index, panel_height);
                    app.sync_process_cwd_to_active_panel_if_no_autosave();

                    let now = std::time::Instant::now();
                    let is_double = app
                        .last_mouse_click
                        .as_ref()
                        .map(|(prev, p, f)| {
                            now.duration_since(*prev) < std::time::Duration::from_millis(400)
                                && *p == panel_index
                                && *f == file_index
                        })
                        .unwrap_or(false);
                    app.last_mouse_click = Some((now, panel_index, file_index));

                    if is_double {
                        let panel = if panel_index == 0 {
                            app.left_panel_mut()
                        } else {
                            app.right_panel_mut()
                        };
                        if let Some(file) = panel.get_selected_file() {
                            let name_lower = file.name.trim_end_matches('/').to_lowercase();
                            let is_archive = name_lower.ends_with(".zip")
                                || name_lower.ends_with(".tar.gz")
                                || name_lower.ends_with(".tgz");
                            if file.is_dir || is_archive {
                                panel.enter_directory()?;
                                app.sync_process_cwd_to_active_panel_if_no_autosave();
                                return Ok(Some(AppAction::PanelNavigated));
                            } else if !file.is_parent_dir() && file.is_executable {
                                let cmd = format!("./{}", file.name);
                                return Ok(Some(AppAction::RunCommand(cmd)));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

fn try_mouse_editor_confirm(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    if !app.editor_confirm_pending {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (col, row) = (mouse_event.column, mouse_event.row);
        let dialog_rect = save_changes_confirm_rect(area);
        if !dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
            return Some(AppAction::EditorConfirmChoice(EditorConfirmChoice::Cancel));
        }
        if let Some(rects) = editor_confirm_option_rects(area) {
            for (opt_rect, choice) in rects {
                if dialog_layout::pointer_in_dialog(col, row, opt_rect) {
                    return Some(AppAction::EditorConfirmChoice(choice));
                }
            }
        }
    }
    Some(AppAction::Continue)
}

fn try_mouse_copy_overwrite(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    if app.copy_overwrite_dialog.is_none() {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (rect, content) = Renderer::overwrite_dialog_layout(area);
        let (col, row) = (mouse_event.column, mouse_event.row);
        if !dialog_layout::pointer_in_dialog(col, row, rect) {
            return Some(AppAction::CopyOverwriteChoice(5));
        }
        if let Some(i) = numbered_option_index(col, row, content, 5) {
            return Some(AppAction::CopyOverwriteChoice(i + 1));
        }
    }
    Some(AppAction::Continue)
}

fn try_mouse_copy_error(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    if app.copy_error_dialog.is_none() {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (rect, content) = Renderer::error_dialog_layout(area);
        let (col, row) = (mouse_event.column, mouse_event.row);
        if !dialog_layout::pointer_in_dialog(col, row, rect) {
            return Some(AppAction::CopyErrorChoice(CopyErrorChoice::Cancel));
        }
        if let Some(opt_row) = numbered_option_index(col, row, content, 3) {
            let error_choice = match opt_row {
                0 => CopyErrorChoice::Ignore,
                1 => CopyErrorChoice::Cancel,
                _ => CopyErrorChoice::IgnoreAll,
            };
            return Some(AppAction::CopyErrorChoice(error_choice));
        }
    }
    Some(AppAction::Continue)
}

fn try_mouse_pattern_select(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    try_mouse_modal_rect_twin_buttons(
        app.pattern_select_dialog.is_some(),
        mouse_event,
        pattern_select_dialog::dialog_rect(area),
        pattern_select_dialog::button_rects(area),
        AppAction::PatternSelectCancel,
        AppAction::PatternSelectConfirm,
        AppAction::PatternSelectCancel,
    )
}

fn try_mouse_mkdir(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    try_mouse_single_input_dialog(
        app,
        area,
        mouse_event,
        app.mkdir_dialog.is_some(),
        |a| {
            a.mkdir_dialog
                .as_ref()
                .map(|d| d.input.text.clone())
                .unwrap_or_default()
        },
        |name| AppAction::MkdirConfirm(name),
        AppAction::MkdirCancel,
    )
}

fn try_mouse_archive(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    try_mouse_single_input_dialog(
        app,
        area,
        mouse_event,
        app.archive_dialog.is_some(),
        |a| {
            a.archive_dialog
                .as_ref()
                .map(|d| d.input.text.clone())
                .unwrap_or_default()
        },
        |name| AppAction::ArchiveConfirm(name),
        AppAction::ArchiveCancel,
    )
}

fn try_mouse_new_file_error(
    app: &mut AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    if app.new_file_error.is_none() {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (col, row) = (mouse_event.column, mouse_event.row);
        let err_rect = Renderer::new_file_error_dialog_rect(area);
        if !dialog_layout::pointer_in_dialog(col, row, err_rect) {
            app.new_file_error = None;
            return Some(AppAction::Continue);
        }
        if let Some(ok_rect) = Renderer::new_file_error_ok_rect(area) {
            if dialog_layout::pointer_in_dialog(col, row, ok_rect) {
                app.new_file_error = None;
            }
        }
    }
    Some(AppAction::Continue)
}

fn try_mouse_new_file(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    try_mouse_single_input_dialog(
        app,
        area,
        mouse_event,
        app.new_file_dialog.is_some(),
        |a| {
            a.new_file_dialog
                .as_ref()
                .map(|d| d.input.text.clone())
                .unwrap_or_default()
        },
        |name| AppAction::NewFileConfirm(name),
        AppAction::NewFileCancel,
    )
}

fn try_mouse_help(
    app: &mut AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    help_dialog::handle_mouse(app, area, mouse_event)
}

fn try_mouse_settings(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    try_modal_left_click_outside_rect(
        app.settings_dialog.is_some(),
        mouse_event,
        settings_dialog::dialog_rect(area),
        AppAction::SettingsClose,
    )
}

fn try_mouse_size_info(
    app: &AppState,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    if app.size_info_dialog.is_none() {
        return None;
    }
    if matches!(mouse_event.kind, MouseEventKind::Down(_)) {
        return Some(AppAction::SizeInfoClose);
    }
    None
}

fn try_mouse_find(
    app: &mut AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    if app.find_dialog.is_none() {
        return None;
    }
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let (col, row) = (mouse_event.column, mouse_event.row);
        if let Some(d) = app.find_dialog.as_ref() {
            let rect = find_dialog::dialog_rect(area, d.phase);
            if !dialog_layout::pointer_in_dialog(col, row, rect) {
                if let Some(a) = find_dialog::pointer_outside_action(app) {
                    return Some(a);
                }
            }
        }
    }
    Some(AppAction::Continue)
}

fn try_mouse_rename_attr(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    try_modal_left_click_outside_rect(
        app.rename_attr_dialog.is_some(),
        mouse_event,
        rename_attr::dialog_rect(area),
        AppAction::RenameAttrCancel,
    )
}

fn try_mouse_operation_confirm(
    app: &AppState,
    area: Rect,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    let (op, _) = app.operation_confirm_pending.as_ref()?;
    if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
        let show_paths = matches!(op, Operation::Copy | Operation::Move);
        if let Some((dialog_rect, yes_rect, no_rect)) =
            Renderer::operation_confirm_button_rects(area, show_paths)
        {
            let (col, row) = (mouse_event.column, mouse_event.row);
            if !dialog_layout::pointer_in_dialog(col, row, dialog_rect) {
                return Some(AppAction::DeleteConfirmChoice(DeleteConfirmChoice::No));
            }
            match hit_test_twin_buttons(col, row, yes_rect, no_rect) {
                Some(TwinButton::Primary) => {
                    return Some(AppAction::DeleteConfirmChoice(DeleteConfirmChoice::Yes));
                }
                Some(TwinButton::Secondary) => {
                    return Some(AppAction::DeleteConfirmChoice(DeleteConfirmChoice::No));
                }
                None => {}
            }
        }
    }
    Some(AppAction::Continue)
}

fn menu_bar_selection_is_plain_file(app: &AppState) -> bool {
    app.active_panel_ref()
        .get_selected_file()
        .map_or(false, |f| !f.is_dir && !f.is_parent_dir())
}

fn menu_bar_copy_or_move_confirm(
    app: &mut AppState,
    op: Operation,
) -> Option<AppAction> {
    debug_assert!(matches!(op, Operation::Copy | Operation::Move));
    let source = app.get_current_dir().to_string();
    let target = app.get_opposite_panel_dir().to_string();
    if source == target && op == Operation::Move {
        app.focus_command_line();
        return None;
    }
    let (names, restore_after, restore_before) = app
        .active_panel_mut()
        .get_names_to_copy_with_restore_neighbors();
    if names.is_empty() {
        if op == Operation::Move {
            app.focus_command_line();
        }
        return None;
    }
    let target_names = if source == target {
        Some(
            names
                .iter()
                .map(|(n, _)| crate::core::copy_state::same_folder_copy_dest_name(n))
                .collect(),
        )
    } else {
        None
    };
    let opposite = app.get_opposite_panel_location();
    let (target_location, target_fs_path) = match &opposite {
        PanelLocation::Archive { .. } => (Some(opposite), None),
        PanelLocation::Fs(_) => (None, Some(app.get_opposite_panel_target_fs_path())),
    };
    let params = CopyParams {
        source_dir: source,
        target_dir: target,
        source_location: Some(app.get_current_location()),
        target_location,
        target_fs_path,
        items: names,
        target_names,
        restore_selection_after: restore_after,
        restore_selection_before: restore_before,
    };
    app.operation_confirm_pending = Some((op, params));
    app.operation_confirm_focus_yes = true;
    Some(AppAction::Continue)
}

fn menu_bar_delete_confirm(app: &mut AppState) -> Option<AppAction> {
    let (names, restore_after, restore_before) = app
        .active_panel_mut()
        .get_names_to_copy_with_restore_neighbors();
    if names.is_empty() {
        return None;
    }
    app.operation_confirm_pending = Some((
        Operation::Delete,
        CopyParams {
            source_dir: app.get_current_dir().to_string(),
            target_dir: String::new(),
            source_location: Some(app.get_current_location()),
            target_location: None,
            target_fs_path: None,
            items: names,
            target_names: None,
            restore_selection_after: restore_after,
            restore_selection_before: restore_before,
        },
    ));
    app.operation_confirm_focus_yes = true;
    Some(AppAction::Continue)
}

/// Map mouse column to menu bar item. Bottom row.
fn hit_test_menu_bar(
    col: u16,
    term_w: u16,
    app: &mut AppState,
) -> Option<AppAction> {
    let items = Renderer::menu_bar_items();
    let menu_item_count = items.len() as u16;
    if menu_item_count == 0 {
        return None;
    }
    let slot_w = term_w / menu_item_count;
    let slot_index = (col / slot_w).min(menu_item_count - 1) as usize;
    let (_, key) = items.get(slot_index)?;
    if app.focus == Focus::CommandLine && *key != menu_bar_key::QUIT {
        return None;
    }
    match *key {
        menu_bar_key::HELP => Some(AppAction::OpenHelpDialog),
        menu_bar_key::FILE => Some(AppAction::OpenRenameAttrDialog),
        menu_bar_key::VIEW => {
            menu_bar_selection_is_plain_file(app).then_some(AppAction::OpenViewer)
        }
        menu_bar_key::EDIT => {
            if supports_edit(&app.get_current_location()) && menu_bar_selection_is_plain_file(app) {
                Some(AppAction::OpenEditor)
            } else {
                None
            }
        }
        menu_bar_key::COPY => menu_bar_copy_or_move_confirm(app, Operation::Copy),
        menu_bar_key::MOVE => menu_bar_copy_or_move_confirm(app, Operation::Move),
        menu_bar_key::FOLDER => {
            if supports_mkdir(&app.get_current_location()) {
                Some(AppAction::OpenMkdirDialog)
            } else {
                None
            }
        }
        menu_bar_key::DELETE => menu_bar_delete_confirm(app),
        menu_bar_key::SETTINGS => Some(AppAction::OpenSettingsDialog),
        menu_bar_key::QUIT => Some(AppAction::Quit),
        _ => None,
    }
}

/// Map mouse (col, row) to (panel_index, file_index) when clicking in a panel's file list.
/// Layout must match [`Renderer::draw_panels_view`] (frame border 1, inner, left/right split).
fn hit_test_panel(
    col: u16,
    row: u16,
    term_w: u16,
    term_h: u16,
    app: &AppState,
) -> Option<(usize, usize)> {
    let content_height = term_h.saturating_sub(2);
    let inner_x = 1u16;
    let inner_y = 1u16;
    let inner_w = term_w.saturating_sub(2);
    let inner_h = content_height.saturating_sub(2);
    let panel_content_height = inner_h.saturating_sub(1);
    let left_w = inner_w / 2;
    let right_w = inner_w.saturating_sub(left_w).saturating_sub(1);

    if row < inner_y || row >= inner_y + panel_content_height {
        return None;
    }
    let local_row = (row - inner_y) as usize;

    let panel_height = compute_panel_height();

    if col >= inner_x && col < inner_x + left_w {
        let panel = app.left_panel();
        let scroll = panel.get_scroll_offset();
        let files_len = panel.get_files().len();
        let file_index = match panel.get_view_mode() {
            ViewMode::SingleColumn => scroll + local_row,
            ViewMode::DoubleColumn => {
                let in_right_col = col >= inner_x + left_w / 2;
                if in_right_col {
                    scroll + panel_height + local_row
                } else {
                    scroll + local_row
                }
            }
        };
        if file_index < files_len {
            return Some((0, file_index));
        }
    }

    if col >= inner_x + left_w + 1 && col < inner_x + left_w + 1 + right_w {
        let panel = app.right_panel();
        let scroll = panel.get_scroll_offset();
        let files_len = panel.get_files().len();
        let right_panel_x = inner_x + left_w + 1;
        let col_in_right = col - right_panel_x;
        let file_index = match panel.get_view_mode() {
            ViewMode::SingleColumn => scroll + local_row,
            ViewMode::DoubleColumn => {
                let in_right_col = col_in_right >= right_w / 2;
                if in_right_col {
                    scroll + panel_height + local_row
                } else {
                    scroll + local_row
                }
            }
        };
        if file_index < files_len {
            return Some((1, file_index));
        }
    }

    None
}
