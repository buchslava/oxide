//! Ctrl+Q / Ctrl+W panel settings overlay. Same options as F9 Settings → Left panel / Right panel.
//! in the F9 Settings dialog, positioned over the respective panel.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::state::AppState;
use crate::app::events::{AppAction, SettingChange};
use crate::dialogs::settings_dialog::draw_panel_section;

const HINT_H: u16 = 1;

/// Open the Left panel settings overlay (Ctrl+Q). Closes the right panel overlay if open.
pub fn open_left(app: &mut AppState) {
    app.right_panel_settings_overlay = None;
    app.left_panel_settings_overlay = Some(Default::default());
}

/// Open the Right panel settings overlay (Ctrl+W). Closes the left panel overlay if open.
pub fn open_right(app: &mut AppState) {
    app.left_panel_settings_overlay = None;
    app.right_panel_settings_overlay = Some(Default::default());
}

/// Close the Left panel overlay.
pub fn close_left(app: &mut AppState) {
    app.left_panel_settings_overlay = None;
}

/// Close the Right panel overlay.
pub fn close_right(app: &mut AppState) {
    app.right_panel_settings_overlay = None;
}

/// Draw the overlay over the given panel area. Centered within the panel rect. is_left: use left panel settings.
fn draw_overlay(
    f: &mut Frame,
    panel_rect: Rect,
    title: &str,
    app: &AppState,
    content_focus: usize,
    is_left: bool,
) {
    // Fill the panel (minus a 1-cell border); the old MIN.min(available) capped width at 28.
    let w = panel_rect.width.saturating_sub(2).max(1);
    let h = panel_rect.height.saturating_sub(2).max(1);
    let x = panel_rect.x + (panel_rect.width.saturating_sub(w)) / 2;
    let y = panel_rect.y + (panel_rect.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    let grey_bg = Color::Rgb(60, 60, 60);
    let right_bg = Color::Rgb(50, 52, 58);
    let fill_style = Style::default().bg(grey_bg).fg(Color::White);
    let right_fill_style = Style::default().bg(right_bg).fg(Color::White);
    let border_style = fill_style.fg(Color::Cyan);

    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::raw(title))
        .style(border_style);
    f.render_widget(block, rect);

    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(HINT_H),
    };

    let persisted = &app.persisted_settings;
    let (view_index, sort_index, dirs_first, show_hidden) = if is_left {
        let view_index = if persisted.left_view.as_str() == "one" {
            1
        } else {
            0
        };
        let sort_index = crate::core::file_ops::SORT_MODES
            .iter()
            .position(|s| *s == persisted.left_sort.as_str())
            .unwrap_or(0);
        (
            view_index,
            sort_index,
            persisted.left_dirs_first,
            persisted.left_show_hidden,
        )
    } else {
        let view_index = if persisted.right_view.as_str() == "one" {
            1
        } else {
            0
        };
        let sort_index = crate::core::file_ops::SORT_MODES
            .iter()
            .position(|s| *s == persisted.right_sort.as_str())
            .unwrap_or(0);
        (
            view_index,
            sort_index,
            persisted.right_dirs_first,
            persisted.right_show_hidden,
        )
    };

    draw_panel_section(
        f,
        content_rect,
        right_fill_style,
        view_index,
        sort_index,
        dirs_first,
        show_hidden,
        content_focus,
    );

    let hint_rect = Rect {
        x: inner.x,
        y: inner.y + content_rect.height,
        width: inner.width,
        height: HINT_H,
    };
    f.render_widget(
        Paragraph::new("↑↓  Tab  Space/Enter  Toggle   Esc  Close")
            .style(fill_style.fg(Color::DarkGray))
            .alignment(Alignment::Center),
        hint_rect,
    );
}

/// Draw whichever panel overlay is open. Call after panels are drawn; uses app.left_panel_rect / right_panel_rect.
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    if let (Some(state), Some(panel_rect)) = (
        app.left_panel_settings_overlay.as_ref(),
        app.left_panel_rect,
    ) {
        draw_overlay(
            f,
            panel_rect,
            " Left panel ",
            app,
            state.content_focus,
            true,
        );
    } else if let (Some(state), Some(panel_rect)) = (
        app.right_panel_settings_overlay.as_ref(),
        app.right_panel_rect,
    ) {
        draw_overlay(
            f,
            panel_rect,
            " Right panel ",
            app,
            state.content_focus,
            false,
        );
    }
}

/// Handle key for the panel overlay. Returns None if no overlay is open.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    let (is_left, state) = if let Some(s) = app.left_panel_settings_overlay.as_mut() {
        (true, s)
    } else if let Some(s) = app.right_panel_settings_overlay.as_mut() {
        (false, s)
    } else {
        return None;
    };

    let persisted_snapshot = app.persisted_settings.clone();

    match code {
        KeyCode::Esc => {
            return Some(if is_left {
                AppAction::CloseLeftPanelSettings
            } else {
                AppAction::CloseRightPanelSettings
            });
        }
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) && c == 'o' => {
            return Some(AppAction::Suspend);
        }
        KeyCode::Tab => {
            state.content_focus = (state.content_focus + 1) % 4;
            return Some(AppAction::Continue);
        }
        KeyCode::Up => {
            if state.content_focus == 1 {
                return Some(AppAction::SettingChange(if is_left {
                    SettingChange::LeftSortCyclePrev
                } else {
                    SettingChange::RightSortCyclePrev
                }));
            }
            if state.content_focus >= 2 {
                state.content_focus -= 1;
            } else {
                return Some(AppAction::SettingChange(if is_left {
                    SettingChange::LeftViewCycle
                } else {
                    SettingChange::RightViewCycle
                }));
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Down => {
            if state.content_focus == 0 {
                if (is_left && persisted_snapshot.left_view.as_str() == "two")
                    || (!is_left && persisted_snapshot.right_view.as_str() == "two")
                {
                    return Some(AppAction::SettingChange(if is_left {
                        SettingChange::LeftViewCycle
                    } else {
                        SettingChange::RightViewCycle
                    }));
                }
                state.content_focus = 1;
            } else if state.content_focus == 1 {
                return Some(AppAction::SettingChange(if is_left {
                    SettingChange::LeftSortCycle
                } else {
                    SettingChange::RightSortCycle
                }));
            } else if state.content_focus < 3 {
                state.content_focus += 1;
            } else {
                state.content_focus = 0;
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let action = match state.content_focus {
                0 => Some(AppAction::SettingChange(if is_left {
                    SettingChange::LeftViewCycle
                } else {
                    SettingChange::RightViewCycle
                })),
                1 => Some(AppAction::SettingChange(if is_left {
                    SettingChange::LeftSortCycle
                } else {
                    SettingChange::RightSortCycle
                })),
                2 => Some(AppAction::SettingChange(if is_left {
                    SettingChange::LeftDirsFirstToggle
                } else {
                    SettingChange::RightDirsFirstToggle
                })),
                _ => Some(AppAction::SettingChange(if is_left {
                    SettingChange::LeftShowHiddenToggle
                } else {
                    SettingChange::RightShowHiddenToggle
                })),
            };
            if let Some(a) = action {
                return Some(a);
            }
        }
        _ => {}
    }
    Some(AppAction::Continue)
}
