//! Apply F9 / panel overlay setting toggles to `AppState` and persist side effects.

use crate::app::state::AppState;
use crate::app::events::SettingChange;
use crate::browser::panel::PanelOperations;
use crate::util;

fn cycle_view_one_two(view: &mut String) {
    *view = if view.as_str() == "one" {
        "two".to_string()
    } else {
        "one".to_string()
    };
}

pub(crate) fn apply_persisted_setting_change(
    app: &mut AppState,
    change: SettingChange,
) {
    match change {
        SettingChange::AutosaveToggle => {
            app.persisted_settings.autosave = !app.persisted_settings.autosave;
        }
        SettingChange::SyncPanelToShellCwdToggle => {
            app.persisted_settings.sync_panel_to_shell_cwd =
                !app.persisted_settings.sync_panel_to_shell_cwd;
        }
        SettingChange::AutoReopenPanelsAfterCommandToggle => {
            app.persisted_settings.auto_reopen_panels_after_command =
                !app.persisted_settings.auto_reopen_panels_after_command;
        }
        SettingChange::AutoReopenPanelsAfterCommandDelayCycle => {
            let cur = app
                .persisted_settings
                .auto_reopen_panels_after_command_delay_secs
                .max(1);
            let max = 30u64;
            let next = if cur >= max { 1 } else { cur + 1 };
            app.persisted_settings
                .auto_reopen_panels_after_command_delay_secs = next;
        }
        SettingChange::LeftViewCycle => cycle_view_one_two(&mut app.persisted_settings.left_view),
        SettingChange::RightViewCycle => cycle_view_one_two(&mut app.persisted_settings.right_view),
        SettingChange::LeftShowHiddenToggle => {
            app.persisted_settings.left_show_hidden = !app.persisted_settings.left_show_hidden;
        }
        SettingChange::RightShowHiddenToggle => {
            app.persisted_settings.right_show_hidden = !app.persisted_settings.right_show_hidden;
        }
        SettingChange::LeftSortCycle => {
            app.persisted_settings.left_sort =
                crate::core::file_ops::cycle_sort_mode(&app.persisted_settings.left_sort, true);
        }
        SettingChange::RightSortCycle => {
            app.persisted_settings.right_sort =
                crate::core::file_ops::cycle_sort_mode(&app.persisted_settings.right_sort, true);
        }
        SettingChange::LeftSortCyclePrev => {
            app.persisted_settings.left_sort =
                crate::core::file_ops::cycle_sort_mode(&app.persisted_settings.left_sort, false);
        }
        SettingChange::RightSortCyclePrev => {
            app.persisted_settings.right_sort =
                crate::core::file_ops::cycle_sort_mode(&app.persisted_settings.right_sort, false);
        }
        SettingChange::LeftDirsFirstToggle => {
            app.persisted_settings.left_dirs_first = !app.persisted_settings.left_dirs_first;
        }
        SettingChange::RightDirsFirstToggle => {
            app.persisted_settings.right_dirs_first = !app.persisted_settings.right_dirs_first;
        }
    }
}

pub(crate) fn setting_change_skips_panel_resync(change: SettingChange) -> bool {
    matches!(
        change,
        SettingChange::AutosaveToggle
            | SettingChange::SyncPanelToShellCwdToggle
            | SettingChange::AutoReopenPanelsAfterCommandToggle
            | SettingChange::AutoReopenPanelsAfterCommandDelayCycle
    )
}

pub(crate) fn toggle_show_hidden_on_active_panel(app: &mut AppState) {
    let panel_height = util::compute_panel_height();
    if app.active_panel() == 0 {
        let new_show = !app.left_panel().get_show_hidden();
        app.left_panel_mut().set_show_hidden(new_show);
        app.persisted_settings.left_show_hidden = new_show;
        app.show_hidden_files = new_show;
        let left_name = app.left_panel().get_selected_file().map(|f| f.name.clone());
        util::log_if_err(
            "Save settings",
            crate::core::settings::save(&app.persisted_settings),
        );
        util::log_if_err(
            "Refresh panel",
            app.left_panel_mut().refresh_files_restore_selection(
                left_name.as_deref(),
                None,
                Some(panel_height),
            ),
        );
    } else {
        let new_show = !app.right_panel().get_show_hidden();
        app.right_panel_mut().set_show_hidden(new_show);
        app.persisted_settings.right_show_hidden = new_show;
        app.show_hidden_files = new_show;
        let right_name = app
            .right_panel()
            .get_selected_file()
            .map(|f| f.name.clone());
        util::log_if_err(
            "Save settings",
            crate::core::settings::save(&app.persisted_settings),
        );
        util::log_if_err(
            "Refresh panel",
            app.right_panel_mut().refresh_files_restore_selection(
                right_name.as_deref(),
                None,
                Some(panel_height),
            ),
        );
    }
}
