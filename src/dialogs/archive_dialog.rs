//! Ctrl+A "Archive" dialog (MC-style). Single text field for the archive file name;
//! selected items are zipped into it; originals are kept. Enter = create (if non-empty), Esc = cancel.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;

use crate::app::state::{AppState, ArchiveMessage, ArchiveProgress};
use crate::core::file_ops::FileOperations;
use crate::core::location::PanelLocation;
use crate::app::events::AppAction;
use crate::ui::text_input::{self, TextInputState};

/// State for Ctrl+A "Archive" dialog. Single text field for the archive file name (e.g. archive.zip).
/// focus: 0 = textarea, 1 = Create, 2 = Cancel.
#[derive(Debug)]
pub struct ArchiveDialogState {
    pub input: TextInputState,
    pub focus: usize,
}

/// Open the dialog with an empty archive name. Call only when at least one item is selected and location is Fs.
pub fn open(app: &mut AppState) {
    app.archive_dialog = Some(ArchiveDialogState {
        input: TextInputState::new(String::new()),
        focus: 0,
    });
}

/// Close the dialog without creating. Called on Esc or Ctrl+C.
pub fn cancel(app: &mut AppState) {
    app.archive_dialog = None;
}

/// Take the entered name and close the dialog. Returns the name (may be empty).
pub fn confirm(app: &mut AppState) -> Option<String> {
    app.archive_dialog.take().map(|d| d.input.text)
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
    if let Err(e) = crate::core::panel_backend::create_archive(&loc, items, name) {
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
pub fn start_archive_background(
    app: &mut AppState,
    name: &str,
    items: &[(String, bool)],
) {
    let loc = app.get_current_location();
    let PanelLocation::Fs(base_dir) = &loc else {
        eprintln!("Archive only supported on filesystem");
        return;
    };
    let items: Vec<(String, bool)> = items.to_vec();
    let name = name.to_string();
    let target_path = FileOperations::join_path(base_dir, &name)
        .to_string_lossy()
        .to_string();

    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));

    let total = items.len();
    let first_path = items
        .first()
        .map(|(n, _)| {
            FileOperations::join_path(base_dir, n)
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_default();
    app.archive_progress = Some(ArchiveProgress {
        current_path: first_path,
        target_path: target_path.clone(),
        current: 1,
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
        let result = crate::core::panel_backend::create_archive_with_progress(
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

impl ArchiveDialogState {
    /// Pure key handler: returns updated state (None = close dialog) and action.
    #[must_use]
    pub fn handle_key(
        self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Option<Self>, AppAction) {
        let (input, focus, result) =
            text_input::handle_single_input_key(self.input, self.focus, code, modifiers);
        match result {
            text_input::SingleInputKeyResult::Confirm => (None, AppAction::ArchiveConfirm),
            text_input::SingleInputKeyResult::Cancel => (None, AppAction::ArchiveCancel),
            text_input::SingleInputKeyResult::Suspend => {
                (Some(Self { input, focus }), AppAction::Suspend)
            }
            text_input::SingleInputKeyResult::Continue => {
                (Some(Self { input, focus }), AppAction::Continue)
            }
        }
    }
}

/// Handle a key when the archive dialog is open. Updates app state by replacement; returns action.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    let d = app.archive_dialog.take()?;
    let (new_dialog, action) = d.handle_key(code, modifiers);
    app.archive_dialog = new_dialog;
    Some(action)
}

/// Draw the "Archive" dialog.
pub fn draw(
    f: &mut ratatui::Frame,
    app: &mut AppState,
) {
    let Some(ref d) = app.archive_dialog else {
        return;
    };
    text_input::draw_single_input_dialog(
        f,
        f.area(),
        " Archive ",
        "Enter archive file name:",
        &d.input,
        d.focus,
        &app.ui_palette,
    );
}

/// Return (create_button_rect, cancel_button_rect) for archive dialog hit-testing.
pub fn archive_button_rects(area: Rect) -> Option<(Rect, Rect)> {
    Some(crate::ui::dialog_layout::single_input_dialog_button_rects(area))
}
