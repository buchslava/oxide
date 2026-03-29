//! Ctrl+N "New file" dialog (MC-style). Single text field for the new file name;
//! Enter = create empty file (if non-empty), Esc = cancel. Works on filesystem and inside ZIP.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app::events::AppAction;
use crate::app::state::AppState;
use crate::core::panel_backend;
use crate::ui::text_input::{self, TextInputState};
use crate::util::compute_panel_height;

/// State for Ctrl+N "New file" dialog. Single text field for the new file name.
/// focus: 0 = textarea, 1 = Create, 2 = Cancel.
#[derive(Debug)]
pub struct NewFileDialogState {
    pub input: TextInputState,
    pub focus: usize,
}

/// Open the dialog with an empty file name.
pub fn open(app: &mut AppState) {
    app.new_file_dialog = Some(NewFileDialogState {
        input: TextInputState::new(String::new()),
        focus: 0,
    });
}

/// Close the dialog without creating. Called on Esc or Ctrl+C.
pub fn cancel(app: &mut AppState) {
    app.new_file_dialog = None;
}

/// Create the empty file in the active panel's current location and refresh the panel.
/// If the file already exists, sets app.new_file_error with a message and leaves the dialog closed.
/// Call only when name is non-empty (after trim). Supported on filesystem and inside ZIP archives.
pub fn create_and_refresh(
    app: &mut AppState,
    name: &str,
) {
    let loc = app.get_current_location();
    if let Ok(true) = panel_backend::entry_exists(&loc, name) {
        app.new_file_error = Some(format!("File already exists: {}", name));
        return;
    }
    if let Err(e) = panel_backend::write_file(&loc, name, &[]) {
        app.new_file_error = Some(format!("{}: {}", name, e));
        return;
    }
    let panel_height = compute_panel_height();
    let _ = app.active_panel_mut().refresh_files_restore_selection(
        Some(name),
        None,
        Some(panel_height),
    );
}

/// Handle a key when the new file dialog is open. On Confirm the entered name is on [`AppAction::NewFileConfirm`].
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    let d = app.new_file_dialog.take()?;
    let (input, focus, result) =
        text_input::handle_single_input_key(d.input, d.focus, code, modifiers);
    match result {
        text_input::SingleInputKeyResult::Confirm => {
            app.new_file_dialog = None;
            Some(AppAction::NewFileConfirm(input.text))
        }
        text_input::SingleInputKeyResult::Cancel => {
            app.new_file_dialog = None;
            Some(AppAction::NewFileCancel)
        }
        text_input::SingleInputKeyResult::Suspend => {
            app.new_file_dialog = Some(NewFileDialogState { input, focus });
            Some(AppAction::Suspend)
        }
        text_input::SingleInputKeyResult::Continue => {
            app.new_file_dialog = Some(NewFileDialogState { input, focus });
            Some(AppAction::Continue)
        }
    }
}

/// Draw the "New file" dialog.
pub fn draw(
    f: &mut ratatui::Frame,
    app: &mut AppState,
) {
    let Some(ref d) = app.new_file_dialog else {
        return;
    };
    text_input::draw_single_input_dialog(
        f,
        f.area(),
        " New file ",
        "Enter file name:",
        &d.input,
        d.focus,
        &app.ui_palette,
    );
}
