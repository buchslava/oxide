//! F7 "Create a new Directory" dialog (MC-style). Single text field for the new folder name;
//! Enter = create (if non-empty), Esc = cancel.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::core::panel_backend;
use crate::app::events::AppAction;
use crate::ui::text_input::{self, TextInputState};

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

/// Take the entered name and close the dialog. Returns the name (may be empty).
pub fn confirm(app: &mut AppState) -> Option<String> {
    app.mkdir_dialog.take().map(|d| d.input.text)
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
    let panel_height = crate::util::compute_panel_height();
    let _ = app.active_panel_mut().refresh_files_restore_selection(
        Some(name),
        None,
        Some(panel_height),
    );
}

impl MkdirDialogState {
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
            text_input::SingleInputKeyResult::Confirm => (None, AppAction::MkdirConfirm),
            text_input::SingleInputKeyResult::Cancel => (None, AppAction::MkdirCancel),
            text_input::SingleInputKeyResult::Suspend => {
                (Some(Self { input, focus }), AppAction::Suspend)
            }
            text_input::SingleInputKeyResult::Continue => {
                (Some(Self { input, focus }), AppAction::Continue)
            }
        }
    }
}

/// Handle a key when the mkdir dialog is open. Updates app state by replacement; returns action.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    let d = app.mkdir_dialog.take()?;
    let (new_dialog, action) = d.handle_key(code, modifiers);
    app.mkdir_dialog = new_dialog;
    Some(action)
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

/// Return (create_button_rect, cancel_button_rect) for mkdir dialog hit-testing.
pub fn mkdir_button_rects(area: Rect) -> Option<(Rect, Rect)> {
    Some(crate::ui::dialog_layout::single_input_dialog_button_rects(area))
}
