//! Ctrl+N "New file" dialog (MC-style). Single text field for the new file name;
//! Enter = create empty file (if non-empty), Esc = cancel. Works on filesystem and inside ZIP.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::core::panel_backend;
use crate::app::events::AppAction;
use crate::ui::text_input::{self, TextInputState};

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

/// Take the entered name and close the dialog. Returns the name (may be empty).
pub fn confirm(app: &mut AppState) -> Option<String> {
    app.new_file_dialog.take().map(|d| d.input.text)
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
    let panel_height = crate::util::compute_panel_height();
    let _ = app.active_panel_mut().refresh_files_restore_selection(
        Some(name),
        None,
        Some(panel_height),
    );
}

impl NewFileDialogState {
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
            text_input::SingleInputKeyResult::Confirm => (None, AppAction::NewFileConfirm),
            text_input::SingleInputKeyResult::Cancel => (None, AppAction::NewFileCancel),
            text_input::SingleInputKeyResult::Suspend => {
                (Some(Self { input, focus }), AppAction::Suspend)
            }
            text_input::SingleInputKeyResult::Continue => {
                (Some(Self { input, focus }), AppAction::Continue)
            }
        }
    }
}

/// Handle a key when the new file dialog is open. Updates app state by replacement; returns action.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    let d = app.new_file_dialog.take()?;
    let (new_dialog, action) = d.handle_key(code, modifiers);
    app.new_file_dialog = new_dialog;
    Some(action)
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
    );
}

/// Return (create_button_rect, cancel_button_rect) for new file dialog hit-testing.
pub fn new_file_button_rects(area: Rect) -> Option<(Rect, Rect)> {
    Some(crate::ui::dialog_layout::single_input_dialog_button_rects(area))
}
