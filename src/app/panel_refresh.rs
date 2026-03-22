//! Refresh file lists after operations; restore selection by neighbor names.

use crate::app::state::AppState;
use crate::browser::panel::{Panel, PanelOperations};
use crate::util;

fn log_refresh_panel_restore(
    panel: &mut Panel,
    context: &str,
    preferred_after: Option<&str>,
    preferred_before: Option<&str>,
    panel_height: Option<usize>,
) {
    util::log_if_err(
        context,
        panel.refresh_files_restore_selection(preferred_after, preferred_before, panel_height),
    );
}

/// Restore source panel selection after copy/move/delete and refresh both panels.
pub(crate) fn restore_source_panel_and_refresh(
    app: &mut AppState,
    source_dir: &str,
    restore_after: Option<&str>,
    restore_before: Option<&str>,
) {
    let panel_height = Some(util::compute_panel_height());
    let is_left_source = app.left_panel().get_current_dir() == source_dir;
    if is_left_source {
        log_refresh_panel_restore(
            app.left_panel_mut(),
            "Refresh left panel",
            restore_after,
            restore_before,
            panel_height,
        );
        log_refresh_panel_restore(
            app.right_panel_mut(),
            "Refresh right panel",
            None,
            None,
            panel_height,
        );
    } else {
        log_refresh_panel_restore(
            app.right_panel_mut(),
            "Refresh right panel",
            restore_after,
            restore_before,
            panel_height,
        );
        log_refresh_panel_restore(
            app.left_panel_mut(),
            "Refresh left panel",
            None,
            None,
            panel_height,
        );
    }
}

pub(crate) fn refresh_both_panels_restore_selection(
    app: &mut AppState,
    left_preferred: Option<&str>,
    right_preferred: Option<&str>,
) {
    util::log_if_err(
        "Refresh panels",
        app.left_panel_mut()
            .refresh_files_restore_selection(left_preferred, None, None),
    );
    util::log_if_err(
        "Refresh panels",
        app.right_panel_mut()
            .refresh_files_restore_selection(right_preferred, None, None),
    );
}

pub(crate) fn refresh_both_panels_full(app: &mut AppState) {
    refresh_both_panels_restore_selection(app, None, None);
}
