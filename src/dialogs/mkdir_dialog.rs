//! F7 "Create a new Directory" dialog (MC-style). Single text field for the new folder name;
//! Enter = create (if non-empty), Esc = cancel.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app::ctrl_x_chord::{self, SuspendChordResult};
use crate::app::events::AppAction;
use crate::app::state::AppState;
use crate::core::panel_backend;
use crate::ui::text_input::{self, TextInputState};
use crate::util::compute_panel_height;

/// State for F7 "Create a new Directory" dialog (MC-style). Single text field for the new folder name.
/// focus: 0 = textarea, 1 = Create, 2 = Cancel.
#[derive(Debug)]
pub struct MkdirDialogState {
    pub input: TextInputState,
    pub focus: usize,
}

/// Open the dialog with an empty folder name.
pub fn open(app: &mut AppState) {
    open_with_name(app, String::new());
}

/// Open the dialog with an optional default name (e.g. from selected file).
pub fn open_with_name(
    app: &mut AppState,
    default_name: String,
) {
    app.mkdir_dialog = Some(MkdirDialogState {
        input: TextInputState::new(default_name),
        focus: 0,
    });
}

/// Close the dialog without creating. Called on Esc or Ctrl+C.
pub fn cancel(app: &mut AppState) {
    app.mkdir_dialog = None;
}

/// Create the directory in the active panel's current location and refresh the panel.
/// Call only when name is non-empty (after trim). Supported on filesystem and inside ZIP archives.
pub fn create_and_refresh(
    app: &mut AppState,
    name: &str,
) {
    let loc = app.get_current_location();
    if let Err(e) = panel_backend::mkdir(&loc, name) {
        eprintln!("Cannot create directory: {}", e);
        return;
    }
    let panel_height = compute_panel_height();
    let _ = app.active_panel_mut().refresh_files_restore_selection(
        Some(name),
        None,
        Some(panel_height),
    );
}

/// Handle a key when the mkdir dialog is open. On Confirm the entered name is on [`AppAction::MkdirConfirm`].
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    if app.mkdir_dialog.is_some() {
        match ctrl_x_chord::poll_suspend_chord(app, code, modifiers) {
            SuspendChordResult::Consumed => return Some(AppAction::Continue),
            SuspendChordResult::SuspendToShell => return Some(AppAction::Suspend),
            SuspendChordResult::NotHandled => {}
        }
    }
    let d = app.mkdir_dialog.take()?;
    let (input, focus, result) =
        text_input::handle_single_input_key(d.input, d.focus, code, modifiers);
    match result {
        text_input::SingleInputKeyResult::Confirm => {
            app.mkdir_dialog = None;
            Some(AppAction::MkdirConfirm(input.text))
        }
        text_input::SingleInputKeyResult::Cancel => {
            app.mkdir_dialog = None;
            Some(AppAction::MkdirCancel)
        }
        text_input::SingleInputKeyResult::Suspend => {
            app.mkdir_dialog = Some(MkdirDialogState { input, focus });
            Some(AppAction::Suspend)
        }
        text_input::SingleInputKeyResult::Continue => {
            app.mkdir_dialog = Some(MkdirDialogState { input, focus });
            Some(AppAction::Continue)
        }
    }
}

/// Draw the "Create a new Directory" dialog.
pub fn draw(
    f: &mut ratatui::Frame,
    app: &mut AppState,
) {
    let Some(ref d) = app.mkdir_dialog else {
        return;
    };
    text_input::draw_single_input_dialog(
        f,
        f.area(),
        " Create a new Directory ",
        "Enter directory name:",
        &d.input,
        d.focus,
        &app.ui_palette,
    );
}
