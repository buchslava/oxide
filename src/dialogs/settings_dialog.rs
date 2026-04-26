//! F9 "Settings" dialog. Two-column: section list (General, panels, Theme, Info) and details.
//! Tab / Shift+Tab switch focus between columns; Esc or click outside closes.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::ctrl_x_chord::{self, SuspendChordResult};
use crate::app::events::{AppAction, SettingChange};
use crate::app::state::AppState;
use crate::core::file_ops::SORT_MODES;
use crate::ui::theme::{DialogPalette, ThemeId};

/// State for F9 Settings dialog. Only UI navigation; all setting values live in PersistedSettings (single source of truth).
#[derive(Debug, Clone)]
pub struct SettingsDialogState {
    /// Selected section index: 0 General, 1 Left, 2 Right, 3 Theme, 4 Info.
    pub selected_section: usize,
    /// true = focus on left list, false = focus on right content.
    pub focus_left: bool,
    /// When focus_left is false and section is Left/Right panel: 0 = View, 1 = Sort, 2 = Folders first, 3 = Show hidden.
    pub content_focus: usize,
}

impl Default for SettingsDialogState {
    fn default() -> Self {
        Self {
            selected_section: 0,
            focus_left: true,
            content_focus: 0,
        }
    }
}

/// Section indices for the Settings dialog sidebar (F1 opens Actions, not Settings).
pub const SETTINGS_SECTIONS: [&str; 5] = [
    "General settings",
    "Left panel",
    "Right panel",
    "Theme",
    "Info",
];

/// Bounding box of the Settings modal (must match [`draw`]).
pub fn dialog_rect(area: Rect) -> Rect {
    const MARGIN: u16 = 4;
    let w = area.width.saturating_sub(MARGIN).max(1);
    let h = area.height.saturating_sub(MARGIN).max(1);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

const VIEW_OPTS: [&str; 2] = ["Two columns", "One column"];

/// Display labels for sort modes (same order as SORT_MODES).
const SORT_OPTIONS: [&str; 6] = [
    "Name asc",
    "Name desc",
    "Size asc",
    "Size desc",
    "Modification date asc",
    "Modification date desc",
];

/// Open the Settings dialog. UI state only; displayed values come from app.persisted_settings.
pub fn open(app: &mut AppState) {
    app.settings_dialog = Some(SettingsDialogState::default());
}

/// Close the dialog.
pub fn close(app: &mut AppState) {
    app.settings_dialog = None;
}

/// Handle a key when the settings dialog is open.
/// Returns SettingChange to apply to persisted_settings (single source of truth); main saves and applies to panels.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    if app.settings_dialog.is_some() {
        match ctrl_x_chord::poll_suspend_chord(app, code, modifiers) {
            SuspendChordResult::Consumed => return Some(AppAction::Continue),
            SuspendChordResult::SuspendToShell => return Some(AppAction::Suspend),
            SuspendChordResult::NotHandled => {}
        }
    }
    let state = app.settings_dialog.as_mut()?;
    if state.selected_section == 3 {
        let n = ThemeId::ALL.len().max(1);
        state.content_focus = state.content_focus.min(n.saturating_sub(1));
    }
    let persisted_snapshot = app.persisted_settings.clone();
    match code {
        KeyCode::Esc => {
            close(app);
            return Some(AppAction::SettingsClose);
        }
        KeyCode::Tab | KeyCode::BackTab => {
            // Only Tab / Shift+Tab switch between the sections list (left) and details (right).
            state.focus_left = !state.focus_left;
            return Some(AppAction::Continue);
        }
        KeyCode::Up => {
            if state.focus_left {
                let section_count = SETTINGS_SECTIONS.len();
                let prev = state.selected_section;
                state.selected_section =
                    (state.selected_section + section_count - 1) % section_count;
                if prev != state.selected_section {
                    state.content_focus = 0;
                }
                return Some(AppAction::Continue);
            }
            // Right column: Up = previous item or move focus up
            match state.selected_section {
                0 => {
                    state.content_focus = (state.content_focus + 6 - 1) % 6;
                }
                1 => {
                    if state.content_focus == 1 {
                        return Some(AppAction::SettingChange(
                            SettingChange::LeftSortCyclePrev,
                        ));
                    }
                    if state.content_focus >= 2 {
                        state.content_focus -= 1;
                    } else {
                        return Some(AppAction::SettingChange(
                            SettingChange::LeftViewCycle,
                        ));
                    }
                }
                2 => {
                    if state.content_focus == 1 {
                        return Some(AppAction::SettingChange(
                            SettingChange::RightSortCyclePrev,
                        ));
                    }
                    if state.content_focus >= 2 {
                        state.content_focus -= 1;
                    } else {
                        return Some(AppAction::SettingChange(
                            SettingChange::RightViewCycle,
                        ));
                    }
                }
                3 => {
                    let n = ThemeId::ALL.len().max(1);
                    state.content_focus = (state.content_focus + n - 1) % n;
                }
                _ => {}
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Down => {
            if state.focus_left {
                let section_count = SETTINGS_SECTIONS.len();
                let prev = state.selected_section;
                state.selected_section = (state.selected_section + 1) % section_count;
                if prev != state.selected_section {
                    state.content_focus = 0;
                }
                return Some(AppAction::Continue);
            }
            // Right column: Down = next item or move focus down
            match state.selected_section {
                0 => {
                    state.content_focus = (state.content_focus + 1) % 6;
                }
                1 => {
                    if state.content_focus == 0 {
                        if persisted_snapshot.left_view.as_str() == "two" {
                            return Some(AppAction::SettingChange(
                                SettingChange::LeftViewCycle,
                            ));
                        }
                        state.content_focus = 1;
                    } else if state.content_focus == 1 {
                        return Some(AppAction::SettingChange(
                            SettingChange::LeftSortCycle,
                        ));
                    } else if state.content_focus < 3 {
                        state.content_focus += 1;
                    } else {
                        state.content_focus = 0;
                    }
                }
                2 => {
                    if state.content_focus == 0 {
                        if persisted_snapshot.right_view.as_str() == "two" {
                            return Some(AppAction::SettingChange(
                                SettingChange::RightViewCycle,
                            ));
                        }
                        state.content_focus = 1;
                    } else if state.content_focus == 1 {
                        return Some(AppAction::SettingChange(
                            SettingChange::RightSortCycle,
                        ));
                    } else if state.content_focus < 3 {
                        state.content_focus += 1;
                    } else {
                        state.content_focus = 0;
                    }
                }
                3 => {
                    let n = ThemeId::ALL.len().max(1);
                    state.content_focus = (state.content_focus + 1) % n;
                }
                _ => {}
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if state.focus_left {
                return Some(AppAction::Continue);
            }
            let action = match state.selected_section {
                0 => match state.content_focus {
                    0 => Some(AppAction::SettingChange(
                        SettingChange::AutosaveToggle,
                    )),
                    1 => Some(AppAction::SettingChange(
                        SettingChange::SyncPanelToShellCwdToggle,
                    )),
                    2 => Some(AppAction::SettingChange(
                        SettingChange::AutoReopenPanelsAfterCommandToggle,
                    )),
                    3 => Some(AppAction::SettingChange(
                        SettingChange::AutoReopenPanelsAfterCommandDelayCycle,
                    )),
                    4 => Some(AppAction::SettingChange(
                        SettingChange::FilePatternModeCycle,
                    )),
                    5 => Some(AppAction::SettingChange(
                        SettingChange::SafeDeleteToggle,
                    )),
                    _ => None,
                },
                1 => match state.content_focus {
                    0 => Some(AppAction::SettingChange(
                        SettingChange::LeftViewCycle,
                    )),
                    1 => Some(AppAction::SettingChange(
                        SettingChange::LeftSortCycle,
                    )),
                    2 => Some(AppAction::SettingChange(
                        SettingChange::LeftDirsFirstToggle,
                    )),
                    _ => Some(AppAction::SettingChange(
                        SettingChange::LeftShowHiddenToggle,
                    )),
                },
                2 => match state.content_focus {
                    0 => Some(AppAction::SettingChange(
                        SettingChange::RightViewCycle,
                    )),
                    1 => Some(AppAction::SettingChange(
                        SettingChange::RightSortCycle,
                    )),
                    2 => Some(AppAction::SettingChange(
                        SettingChange::RightDirsFirstToggle,
                    )),
                    _ => Some(AppAction::SettingChange(
                        SettingChange::RightShowHiddenToggle,
                    )),
                },
                3 => Some(AppAction::SettingChange(
                    SettingChange::ThemeSelect(state.content_focus),
                )),
                _ => None,
            };
            if let Some(a) = action {
                return Some(a);
            }
        }
        _ => {}
    }
    Some(AppAction::Continue)
}

fn draw_theme_section(
    f: &mut Frame,
    dialog: &DialogPalette,
    area: Rect,
    fill_style: Style,
    persisted_theme_slug: &str,
    content_focus: usize,
) {
    let view_highlight = dialog
        .list_highlight_style()
        .remove_modifier(Modifier::BOLD);
    let current = ThemeId::from_slug(persisted_theme_slug);
    let selected_idx = ThemeId::ALL.iter().position(|&t| t == current).unwrap_or(0);
    let mut y = area.y;
    let line_h = 1u16;

    f.render_widget(
        Paragraph::new(Line::from(Span::raw("Current theme:"))).style(fill_style),
        Rect {
            x: area.x,
            y,
            width: area.width,
            height: line_h,
        },
    );
    y += line_h;

    for (i, &theme) in ThemeId::ALL.iter().enumerate() {
        let sym = if i == selected_idx { "◉ " } else { "○ " };
        let style = if i == content_focus {
            view_highlight
        } else {
            fill_style
        };
        let line = Line::from(vec![
            Span::raw(sym),
            Span::raw(theme.display_name()),
        ]);
        f.render_widget(
            Paragraph::new(line).style(style),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: line_h,
            },
        );
        y += line_h;
    }
}

/// Info text for the Info section: app description, license, link.
fn info_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("OXIDE"),
        Line::from("A New File Manager"),
        Line::from("in the Spirit of the Classics"),
        Line::from(""),
        Line::from("Efficient, fast, and robust"),
        Line::from(""),
        Line::from("MIT License"),
        Line::from("Copyright (c) 2026 Vyacheslav Chub (vyacheslav.chub@gmail.com)"),
        Line::from(""),
        Line::from("https://github.com/buchslava/oxide"),
    ]
}

/// Draw the Settings dialog: two-column layout.
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    let state = match &app.settings_dialog {
        Some(s) => s,
        None => return,
    };

    let area = f.area();
    let rect = dialog_rect(area);
    let d = &app.ui_palette.dialog;
    let fill_style = d.fill_style();
    let right_fill_style = d.fill_secondary_style();
    let border_style = fill_style.fg(d.border);
    let highlight_style = d.list_highlight_style();

    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Settings ")
        .style(border_style);
    f.render_widget(block, rect);

    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let hint_h = 1u16;
    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(hint_h),
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Min(24)])
        .split(content_rect);

    let left_area = chunks[0];
    let right_area = chunks[1];

    let accent = d.accent;
    let muted = d.text_muted;
    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(if state.focus_left {
            Line::from(Span::styled(
                " Sections ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::styled(
                " Sections ",
                Style::default().fg(muted),
            ))
        })
        .border_style(Style::default().fg(if state.focus_left { accent } else { muted }))
        .style(Style::default().bg(d.dialog_bg));

    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(if !state.focus_left {
            Line::from(Span::styled(
                " Details ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::styled(
                " Details ",
                Style::default().fg(muted),
            ))
        })
        .border_style(Style::default().fg(if state.focus_left { muted } else { accent }))
        .style(Style::default().bg(d.dialog_bg_secondary));

    let right_inner = right_block.inner(right_area);
    let fill_w = right_inner.width.max(1) as usize;
    let fill_h = right_inner.height.max(1) as usize;
    let fill_right = Paragraph::new(
        std::iter::repeat(Line::from(Span::raw(" ".repeat(fill_w))))
            .take(fill_h)
            .collect::<Vec<_>>(),
    )
    .style(right_fill_style)
    .block(right_block);
    f.render_widget(fill_right, right_area);

    // Left column: section list
    let list_items: Vec<ListItem> = SETTINGS_SECTIONS
        .iter()
        .map(|s| ListItem::new(*s).style(fill_style))
        .collect();
    let list = List::new(list_items)
        .block(left_block)
        .highlight_style(highlight_style)
        .highlight_symbol("▸ ");
    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_section));
    f.render_stateful_widget(list, left_area, &mut list_state);

    let persisted = &app.persisted_settings;
    let left_view_index = if persisted.left_view.as_str() == "one" {
        1
    } else {
        0
    };
    let right_view_index = if persisted.right_view.as_str() == "one" {
        1
    } else {
        0
    };
    let left_sort_index = SORT_MODES
        .iter()
        .position(|s| *s == persisted.left_sort.as_str())
        .unwrap_or(0);
    let right_sort_index = SORT_MODES
        .iter()
        .position(|s| *s == persisted.right_sort.as_str())
        .unwrap_or(0);
    match state.selected_section {
        0 => {
            let view_highlight = d.list_highlight_style().remove_modifier(Modifier::BOLD);
            let line_h = 1u16;
            let chk0 = if persisted.autosave { "[x]" } else { "[ ]" };
            let style0 = if state.content_focus == 0 {
                view_highlight
            } else {
                right_fill_style
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(chk0),
                    Span::raw(" Autosave latest state"),
                ]))
                .style(style0),
                Rect {
                    x: right_inner.x,
                    y: right_inner.y,
                    width: right_inner.width,
                    height: line_h,
                },
            );
            let chk1 = if persisted.sync_panel_to_shell_cwd {
                "[x]"
            } else {
                "[ ]"
            };
            let style1 = if state.content_focus == 1 {
                view_highlight
            } else {
                right_fill_style
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(chk1),
                    Span::raw(" Sync panel to shell dir when returning (Ctrl+O or Ctrl+X O)"),
                ]))
                .style(style1),
                Rect {
                    x: right_inner.x,
                    y: right_inner.y + line_h,
                    width: right_inner.width,
                    height: line_h,
                },
            );
            let chk2 = if persisted.auto_reopen_panels_after_command {
                "[x]"
            } else {
                "[ ]"
            };
            let style2 = if state.content_focus == 2 {
                view_highlight
            } else {
                right_fill_style
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(chk2),
                    Span::raw(" Auto reopen panels after command/executable"),
                ]))
                .style(style2),
                Rect {
                    x: right_inner.x,
                    y: right_inner.y + 2 * line_h,
                    width: right_inner.width,
                    height: line_h,
                },
            );
            let delay_style = if state.content_focus == 3 {
                view_highlight
            } else {
                right_fill_style
            };
            let delay_secs = persisted.auto_reopen_panels_after_command_delay_secs;
            let delay_label = format!(" Delay (sec): {delay_secs}");
            f.render_widget(
                Paragraph::new(Line::from(delay_label)).style(delay_style),
                Rect {
                    x: right_inner.x,
                    y: right_inner.y + 3 * line_h,
                    width: right_inner.width,
                    height: line_h,
                },
            );
            let pattern_style = if state.content_focus == 4 {
                view_highlight
            } else {
                right_fill_style
            };
            let pattern_label = if persisted.file_pattern_uses_regex() {
                " Find / +/− file pattern: Regular expression"
            } else {
                " Find / +/− file pattern: Wildcards (*, ?)"
            };
            f.render_widget(
                Paragraph::new(Line::from(pattern_label)).style(pattern_style),
                Rect {
                    x: right_inner.x,
                    y: right_inner.y + 4 * line_h,
                    width: right_inner.width,
                    height: line_h,
                },
            );
            let safe_style = if !app.trash_available {
                right_fill_style.fg(d.text_muted)
            } else if state.content_focus == 5 {
                view_highlight
            } else {
                right_fill_style
            };
            let safe_chk = if app.trash_available && persisted.safe_delete {
                "[x]"
            } else {
                "[ ]"
            };
            let safe_suffix = if app.trash_available {
                " Safe delete"
            } else {
                " Safe delete (OS trash unavailable)"
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(safe_chk),
                    Span::raw(safe_suffix),
                ]))
                .style(safe_style),
                Rect {
                    x: right_inner.x,
                    y: right_inner.y + 5 * line_h,
                    width: right_inner.width,
                    height: line_h,
                },
            );
        }
        1 => draw_panel_section(
            f,
            d,
            right_inner,
            right_fill_style,
            left_view_index,
            left_sort_index,
            persisted.left_dirs_first,
            persisted.left_show_hidden,
            state.content_focus,
        ),
        2 => draw_panel_section(
            f,
            d,
            right_inner,
            right_fill_style,
            right_view_index,
            right_sort_index,
            persisted.right_dirs_first,
            persisted.right_show_hidden,
            state.content_focus,
        ),
        3 => draw_theme_section(
            f,
            d,
            right_inner,
            right_fill_style,
            persisted.theme.as_str(),
            state.content_focus,
        ),
        4 => {
            let para = Paragraph::new(info_lines())
                .style(right_fill_style)
                .wrap(Wrap { trim: true });
            f.render_widget(para, right_inner);
        }
        _ => {}
    }

    let hint_rect = Rect {
        x: inner.x,
        y: inner.y + content_rect.height,
        width: inner.width,
        height: hint_h,
    };
    f.render_widget(
        Paragraph::new("↑↓ Navigate  Tab  Sections ↔ Details  Space/Enter  Toggle  Esc  Close")
            .style(fill_style.fg(d.text_muted))
            .alignment(Alignment::Center),
        hint_rect,
    );
}

/// Draw panel options (View, Sort, Folders first, Show hidden). Used by F9 Settings and by panel overlay.
pub(crate) fn draw_panel_section(
    f: &mut Frame,
    dialog: &DialogPalette,
    area: Rect,
    fill_style: Style,
    view_index: usize,
    sort_index: usize,
    dirs_first: bool,
    show_hidden: bool,
    content_focus: usize,
) {
    let view_highlight = dialog
        .list_highlight_style()
        .remove_modifier(Modifier::BOLD);
    let mut y = area.y;
    let line_h = 1u16;

    // View: listbox
    let view_label = Line::from(Span::raw("View:"));
    f.render_widget(
        Paragraph::new(view_label).style(fill_style),
        Rect {
            x: area.x,
            y,
            width: area.width,
            height: line_h,
        },
    );
    y += line_h;

    for (i, opt) in VIEW_OPTS.iter().enumerate() {
        let (sym, style) = if i == view_index {
            (
                "◉ ",
                if content_focus == 0 {
                    view_highlight
                } else {
                    fill_style
                },
            )
        } else {
            ("○ ", fill_style)
        };
        let line = Line::from(vec![Span::raw(sym), Span::raw(*opt)]);
        f.render_widget(
            Paragraph::new(line).style(style),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: line_h,
            },
        );
        y += line_h;
    }

    y += 1;

    // Sort: listbox
    let sort_label = Line::from(Span::raw("Sort:"));
    f.render_widget(
        Paragraph::new(sort_label).style(fill_style),
        Rect {
            x: area.x,
            y,
            width: area.width,
            height: line_h,
        },
    );
    y += line_h;

    for (i, opt) in SORT_OPTIONS.iter().enumerate() {
        let (sym, style) = if i == sort_index {
            (
                "◉ ",
                if content_focus == 1 {
                    view_highlight
                } else {
                    fill_style
                },
            )
        } else {
            ("○ ", fill_style)
        };
        let line = Line::from(vec![Span::raw(sym), Span::raw(*opt)]);
        f.render_widget(
            Paragraph::new(line).style(style),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: line_h,
            },
        );
        y += line_h;
    }

    y += 1;

    // Folders first (directories listed before files)
    let dirs_first_chk = if dirs_first { "[x]" } else { "[ ]" };
    let dirs_first_style = if content_focus == 2 {
        view_highlight
    } else {
        fill_style
    };
    let dirs_first_line = Line::from(vec![
        Span::raw(dirs_first_chk),
        Span::raw(" Folders first"),
    ]);
    f.render_widget(
        Paragraph::new(dirs_first_line).style(dirs_first_style),
        Rect {
            x: area.x,
            y,
            width: area.width,
            height: line_h,
        },
    );
    y += line_h;

    let checkbox = if show_hidden { "[x]" } else { "[ ]" };
    let chk_style = if content_focus == 3 {
        view_highlight
    } else {
        fill_style
    };
    let chk_line = Line::from(vec![
        Span::raw(checkbox),
        Span::raw(" Show hidden files"),
    ]);
    f.render_widget(
        Paragraph::new(chk_line).style(chk_style),
        Rect {
            x: area.x,
            y,
            width: area.width,
            height: line_h,
        },
    );
}
