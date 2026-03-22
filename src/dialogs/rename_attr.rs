//! F2 "Rename / Attributes" dialog. Permissions as 12 checkboxes (Chmod-style); owner/group as list selection (Chown-style).
//! Single file: editable name + checkboxes + user/group lists. Group: checkboxes + user/group lists only.

use std::path::Path;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app::state::AppState;
use crate::browser::clipboard;
use crate::core::file_ops::FileOperations;
use crate::app::events::AppAction;
use crate::browser::panel::PanelOperations;
use crate::ui::styles::{
    DIALOG_BG, DIALOG_INPUT_BG_FOCUSED, DIALOG_INPUT_BG_UNFOCUSED, DIALOG_INPUT_SELECTION_BG,
};
use crate::ui::text_input::{self, TextInputState};

/// Which part of the F2 dialog has focus (name field, permission checkboxes, user list, or group list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameAttrField {
    Name,
    Permissions,
    User,
    Group,
}

/// State for F2 "Rename / Attributes" dialog. Permissions as 12 checkboxes; owner/group as list selection.
#[derive(Debug)]
pub enum RenameAttrDialogState {
    Single {
        name_input: TextInputState,
        /// Unix mode (0o7777: suid, sgid, sticky + rwx).
        mode: u32,
        /// Index into the 12 permission checkboxes (0..12).
        perm_focus: usize,
        owner: String,
        group: String,
        user_list: Vec<String>,
        group_list: Vec<String>,
        user_index: usize,
        group_index: usize,
        old_name: Arc<str>,
        focus: RenameAttrField,
    },
    Group {
        count: usize,
        mode: u32,
        perm_focus: usize,
        owner: String,
        group: String,
        user_list: Vec<String>,
        group_list: Vec<String>,
        user_index: usize,
        group_index: usize,
        items: Vec<(String, bool)>,
        /// Current file name (cursor row when dialog opened); restore selection to it after apply.
        current_name: String,
        focus: RenameAttrField,
    },
}

/// Permission bit masks (0o4000 .. 0o1) and labels for the 12 checkboxes.
const PERM_BITS: [u32; 12] = [
    0o4000, 0o2000, 0o1000, 0o400, 0o200, 0o100, 0o40, 0o20, 0o10, 0o4, 0o2, 0o1,
];
const PERM_LABELS: [&str; 12] = [
    "set user ID on execution",
    "set group ID on execution",
    "sticky bit",
    "read by owner",
    "write by owner",
    "execute/search by owner",
    "read by group",
    "write by group",
    "execute/search by group",
    "read by others",
    "write by others",
    "execute/search by others",
];

fn mode_has_bit(
    mode: u32,
    bit: u32,
) -> bool {
    (mode & bit) != 0
}

fn toggle_perm_bit(
    mode: u32,
    bit: u32,
) -> u32 {
    mode ^ bit
}

/// Open F2 dialog: single file (name + attrs) or group (attrs only). No-op if no selection.
pub fn open(app: &mut AppState) {
    let (items, _, _) = app
        .active_panel_ref()
        .get_names_to_copy_with_restore_neighbors();
    if items.is_empty() {
        return;
    }
    let files = app.active_panel_ref().get_files();
    let cwd = app.get_current_dir().to_string();
    let first_name = items[0].0.trim_end_matches('/');
    let first_file = files
        .iter()
        .find(|f| f.name.trim_end_matches('/') == first_name);

    let path_first = Path::new(&cwd).join(first_name);
    let mode = FileOperations::get_file_mode(&path_first).unwrap_or(0o644) & 0o7777;
    let mut user_list = FileOperations::load_user_list();
    let mut group_list = FileOperations::load_group_list();
    let (owner, group) = first_file
        .map(|f| (f.owner.clone(), f.group.clone()))
        .unwrap_or_else(|| (String::new(), String::new()));
    // Ensure file's owner/group are in the lists (e.g. numeric uid/gid when not in passwd/group)
    let user_index = if owner.is_empty() {
        0
    } else if let Some(i) = user_list.iter().position(|u| u == &owner) {
        i
    } else {
        user_list.insert(0, owner.clone());
        0
    };
    let group_index = if group.is_empty() {
        0
    } else if let Some(i) = group_list.iter().position(|g| g == &group) {
        i
    } else {
        group_list.insert(0, group.clone());
        0
    };

    if items.len() == 1 {
        let name = items[0].0.trim_end_matches('/').to_string();
        let name_arc: Arc<str> = Arc::from(name);
        app.rename_attr_dialog = Some(RenameAttrDialogState::Single {
            name_input: TextInputState::new(name_arc.to_string()),
            mode,
            perm_focus: 0,
            owner,
            group,
            user_list,
            group_list,
            user_index,
            group_index,
            old_name: name_arc,
            focus: RenameAttrField::Name,
        });
    } else {
        let current_name = app
            .active_panel_ref()
            .get_selected_file()
            .map(|f| f.name.trim_end_matches('/').to_string())
            .unwrap_or_else(|| items[0].0.trim_end_matches('/').to_string());
        app.rename_attr_dialog = Some(RenameAttrDialogState::Group {
            count: items.len(),
            mode,
            perm_focus: 0,
            owner,
            group,
            user_list,
            group_list,
            user_index,
            group_index,
            items,
            current_name,
            focus: RenameAttrField::Permissions,
        });
    }
}

/// Close the dialog without applying.
pub fn close_dialog(app: &mut AppState) {
    app.rename_attr_dialog = None;
    app.rename_attr_error = None;
}

/// Close the dialog without applying (alias for close_dialog for cancel flow).
pub fn cancel(app: &mut AppState) {
    close_dialog(app);
}

/// Apply rename (if single and name changed), chmod, and chown; close dialog on success.
/// On error, restores dialog state and sets rename_attr_error for alert. Returns true if panel should refresh.
pub fn apply(app: &mut AppState) -> bool {
    let state = match app.rename_attr_dialog.take() {
        Some(s) => s,
        None => return false,
    };
    let cwd = app.get_current_dir().to_string();
    let panel_height = crate::util::compute_panel_height();

    match state {
        RenameAttrDialogState::Single {
            name_input,
            mode,
            perm_focus,
            owner,
            group,
            user_list,
            group_list,
            user_index,
            group_index,
            old_name,
            focus,
        } => {
            let name_trimmed = name_input.text.trim().to_string();
            let old_name_trim = old_name.as_ref().trim_end_matches('/');
            if !name_trimmed.is_empty() && old_name_trim != name_trimmed {
                let path_old = Path::new(&cwd).join(old_name_trim);
                let path_new = Path::new(&cwd).join(&name_trimmed);
                if let Err(e) = std::fs::rename(&path_old, &path_new) {
                    app.rename_attr_dialog = Some(RenameAttrDialogState::Single {
                        name_input,
                        mode,
                        perm_focus,
                        owner,
                        group,
                        user_list,
                        group_list,
                        user_index,
                        group_index,
                        old_name,
                        focus,
                    });
                    app.rename_attr_error = Some(format!("Rename failed: {}", e));
                    return false;
                }
            }
            let path = Path::new(&cwd).join(if name_trimmed.is_empty() {
                old_name_trim
            } else {
                name_trimmed.as_str()
            });
            if let Err(e) = FileOperations::set_permissions(&path, mode) {
                app.rename_attr_dialog = Some(RenameAttrDialogState::Single {
                    name_input,
                    mode,
                    perm_focus,
                    owner,
                    group,
                    user_list,
                    group_list,
                    user_index,
                    group_index,
                    old_name,
                    focus,
                });
                app.rename_attr_error = Some(format!("Set permissions failed: {}", e));
                return false;
            }
            if let (Some(u), Some(g)) = (user_list.get(user_index), group_list.get(group_index)) {
                if let Err(e) = FileOperations::chown(&path, u, g) {
                    app.rename_attr_dialog = Some(RenameAttrDialogState::Single {
                        name_input,
                        mode,
                        perm_focus,
                        owner,
                        group,
                        user_list,
                        group_list,
                        user_index,
                        group_index,
                        old_name,
                        focus,
                    });
                    app.rename_attr_error = Some(format!("Chown failed: {}", e));
                    return false;
                }
            }
            let _ = app.active_panel_mut().refresh_files_restore_selection(
                Some(if name_trimmed.is_empty() {
                    old_name_trim
                } else {
                    &name_trimmed
                }),
                None,
                Some(panel_height),
            );
            true
        }
        RenameAttrDialogState::Group {
            count,
            mode,
            perm_focus,
            owner,
            group,
            user_list,
            group_list,
            user_index,
            group_index,
            items,
            current_name,
            focus,
        } => {
            let u_g = user_list.get(user_index).zip(group_list.get(group_index));
            for (name, _) in &items {
                let path = Path::new(&cwd).join(name.trim_end_matches('/'));
                if let Err(e) = FileOperations::set_permissions(&path, mode) {
                    app.rename_attr_dialog = Some(RenameAttrDialogState::Group {
                        count,
                        mode,
                        perm_focus,
                        owner,
                        group,
                        user_list,
                        group_list,
                        user_index,
                        group_index,
                        items: items.clone(),
                        current_name: current_name.clone(),
                        focus,
                    });
                    app.rename_attr_error =
                        Some(format!("Set permissions failed: {}: {}", name, e));
                    return false;
                }
                if let Some((u, g)) = u_g {
                    if let Err(e) = FileOperations::chown(&path, u, g) {
                        app.rename_attr_dialog = Some(RenameAttrDialogState::Group {
                            count,
                            mode,
                            perm_focus,
                            owner,
                            group,
                            user_list,
                            group_list,
                            user_index,
                            group_index,
                            items: items.clone(),
                            current_name: current_name.clone(),
                            focus,
                        });
                        app.rename_attr_error = Some(format!("Chown failed: {}: {}", name, e));
                        return false;
                    }
                }
            }
            let panel_height = crate::util::compute_panel_height();
            let _ = app.active_panel_mut().refresh_files_restore_selection(
                Some(current_name.as_str()),
                None,
                Some(panel_height),
            );
            true
        }
    }
}

/// Handle key when F2 dialog is open.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    let d = app.rename_attr_dialog.as_mut()?;
    let code = match code {
        KeyCode::Char('\t') => KeyCode::Tab,
        other => other,
    };
    match code {
        KeyCode::Tab => {
            let next = match d {
                RenameAttrDialogState::Single { focus, .. } => match focus {
                    RenameAttrField::Name => RenameAttrField::Permissions,
                    RenameAttrField::Permissions => RenameAttrField::User,
                    RenameAttrField::User => RenameAttrField::Group,
                    RenameAttrField::Group => RenameAttrField::Name,
                },
                RenameAttrDialogState::Group { focus, .. } => match focus {
                    RenameAttrField::Permissions => RenameAttrField::User,
                    RenameAttrField::User => RenameAttrField::Group,
                    RenameAttrField::Group => RenameAttrField::Permissions,
                    RenameAttrField::Name => RenameAttrField::Permissions,
                },
            };
            match d {
                RenameAttrDialogState::Single { focus, .. } => *focus = next,
                RenameAttrDialogState::Group { focus, .. } => *focus = next,
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Char(c) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                if c == 'v' {
                    if let RenameAttrDialogState::Single {
                        name_input, focus, ..
                    } = d
                    {
                        if *focus == RenameAttrField::Name {
                            if let Some(s) = clipboard::get() {
                                *name_input = std::mem::take(name_input).insert_str(&s);
                            }
                            return Some(AppAction::Continue);
                        }
                    }
                }
                if c == 'c' {
                    if let RenameAttrDialogState::Single {
                        name_input, focus, ..
                    } = d
                    {
                        if *focus == RenameAttrField::Name {
                            let text = name_input
                                .get_selected_text()
                                .unwrap_or_else(|| name_input.text.clone());
                            if !text.is_empty() {
                                clipboard::set(&text);
                                return Some(AppAction::Continue);
                            }
                        }
                    }
                    cancel(app);
                    return Some(AppAction::RenameAttrCancel);
                }
                if c == 'a' {
                    if let RenameAttrDialogState::Single {
                        name_input, focus, ..
                    } = d
                    {
                        if *focus == RenameAttrField::Name && !name_input.text.is_empty() {
                            *name_input = std::mem::take(name_input).select_all();
                            return Some(AppAction::Continue);
                        }
                    }
                }
                if c == 'o' {
                    return Some(AppAction::Suspend);
                }
            }
            if c == ' ' {
                let is_perm = match d {
                    RenameAttrDialogState::Single {
                        focus,
                        mode,
                        perm_focus,
                        ..
                    } => {
                        if *focus == RenameAttrField::Permissions && *perm_focus < 12 {
                            let bit = PERM_BITS[*perm_focus];
                            *mode = toggle_perm_bit(*mode, bit);
                        }
                        *focus == RenameAttrField::Permissions
                    }
                    RenameAttrDialogState::Group {
                        focus,
                        mode,
                        perm_focus,
                        ..
                    } => {
                        if *focus == RenameAttrField::Permissions && *perm_focus < 12 {
                            let bit = PERM_BITS[*perm_focus];
                            *mode = toggle_perm_bit(*mode, bit);
                        }
                        *focus == RenameAttrField::Permissions
                    }
                };
                if !is_perm {
                    return Some(AppAction::RenameAttrConfirm);
                }
            } else if c.is_ascii() && !c.is_control() {
                if let RenameAttrDialogState::Single {
                    name_input, focus, ..
                } = d
                {
                    if *focus == RenameAttrField::Name {
                        *name_input = std::mem::take(name_input).insert_char(c);
                    }
                }
            }
        }
        KeyCode::Backspace => {
            if let RenameAttrDialogState::Single {
                name_input, focus, ..
            } = d
            {
                if *focus == RenameAttrField::Name {
                    *name_input = std::mem::take(name_input).backspace();
                }
            }
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
            if let RenameAttrDialogState::Single {
                name_input, focus, ..
            } = d
            {
                if *focus == RenameAttrField::Name {
                    let shift = modifiers.contains(KeyModifiers::SHIFT);
                    *name_input = match code {
                        KeyCode::Left => std::mem::take(name_input).move_left(shift),
                        KeyCode::Right => std::mem::take(name_input).move_right(shift),
                        KeyCode::Home => std::mem::take(name_input).move_home(shift),
                        KeyCode::End => std::mem::take(name_input).move_end(shift),
                        _ => return Some(AppAction::Continue),
                    };
                }
            }
        }
        KeyCode::Up => match d {
            RenameAttrDialogState::Single {
                perm_focus,
                user_index,
                group_index,
                focus,
                user_list,
                group_list,
                ..
            } => match focus {
                RenameAttrField::Permissions => *perm_focus = perm_focus.saturating_sub(1),
                RenameAttrField::User => {
                    *user_index = user_index
                        .saturating_sub(1)
                        .min(user_list.len().saturating_sub(1).max(0))
                }
                RenameAttrField::Group => {
                    *group_index = group_index
                        .saturating_sub(1)
                        .min(group_list.len().saturating_sub(1).max(0))
                }
                RenameAttrField::Name => {}
            },
            RenameAttrDialogState::Group {
                perm_focus,
                user_index,
                group_index,
                focus,
                user_list,
                group_list,
                ..
            } => match focus {
                RenameAttrField::Permissions => *perm_focus = perm_focus.saturating_sub(1),
                RenameAttrField::User => {
                    *user_index = user_index
                        .saturating_sub(1)
                        .min(user_list.len().saturating_sub(1).max(0))
                }
                RenameAttrField::Group => {
                    *group_index = group_index
                        .saturating_sub(1)
                        .min(group_list.len().saturating_sub(1).max(0))
                }
                RenameAttrField::Name => {}
            },
        },
        KeyCode::Down => match d {
            RenameAttrDialogState::Single {
                perm_focus,
                user_index,
                group_index,
                focus,
                user_list,
                group_list,
                ..
            } => match focus {
                RenameAttrField::Permissions => *perm_focus = (*perm_focus + 1).min(11),
                RenameAttrField::User => {
                    *user_index = (*user_index + 1).min(user_list.len().saturating_sub(1))
                }
                RenameAttrField::Group => {
                    *group_index = (*group_index + 1).min(group_list.len().saturating_sub(1))
                }
                RenameAttrField::Name => {}
            },
            RenameAttrDialogState::Group {
                perm_focus,
                user_index,
                group_index,
                focus,
                user_list,
                group_list,
                ..
            } => match focus {
                RenameAttrField::Permissions => *perm_focus = (*perm_focus + 1).min(11),
                RenameAttrField::User => {
                    *user_index = (*user_index + 1).min(user_list.len().saturating_sub(1))
                }
                RenameAttrField::Group => {
                    *group_index = (*group_index + 1).min(group_list.len().saturating_sub(1))
                }
                RenameAttrField::Name => {}
            },
        },
        KeyCode::Enter => {
            // Enter runs the operation (confirm). Space toggles the permission checkbox.
            return Some(AppAction::RenameAttrConfirm);
        }
        KeyCode::Esc => return Some(AppAction::RenameAttrCancel),
        _ => {}
    }
    Some(AppAction::Continue)
}

/// Draw the F2 dialog. Order left to right: 1 File name, 2 Permissions (in File section), 3 User name, 4 Group name.
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    let d = match app.rename_attr_dialog.as_mut() {
        Some(x) => x,
        None => return,
    };
    let area = f.area();
    let max_w = 72u16;
    let w = max_w.min(area.width.saturating_sub(4));
    let h = 22u16.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    let fill_style = Style::default().bg(DIALOG_BG).fg(Color::White);
    let cyan = Style::default().fg(Color::Cyan);
    let focus_border = Style::default().fg(Color::Yellow);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Rename / Attributes ")
        .style(fill_style.fg(Color::Cyan));
    f.render_widget(block, rect);
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    // Order: 1 File name (top left), 2 Permission (under File name), 3 Owner name (right), 4 Group name (right)
    let left_w = (inner.width / 2).max(32);
    let right_w = inner.width.saturating_sub(left_w).saturating_sub(1);
    let gap = 1u16;

    let focus = match d {
        RenameAttrDialogState::Single { focus, .. } => *focus,
        RenameAttrDialogState::Group { focus, .. } => *focus,
    };

    // 1. File name (top left) — no "Name" label
    let name_block_h = 1u16 + 2; // 1 content row + border
    let name_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: left_w,
        height: name_block_h,
    };
    let name_inner = name_rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    match d {
        RenameAttrDialogState::Single {
            name_input,
            focus: _,
            ..
        } => {
            let name_input_bg = if focus == RenameAttrField::Name {
                DIALOG_INPUT_BG_FOCUSED
            } else {
                DIALOG_INPUT_BG_UNFOCUSED
            };
            let name_rect = Rect {
                x: name_inner.x,
                y: name_inner.y,
                width: name_inner.width,
                height: 1,
            };
            let base_style = fill_style.bg(name_input_bg).fg(Color::White);
            let selection_style = Style::default()
                .bg(DIALOG_INPUT_SELECTION_BG)
                .fg(Color::White);
            let line = text_input::input_line_with_selection(
                name_input,
                name_rect.width as usize,
                base_style,
                selection_style,
            );
            f.render_widget(Paragraph::new(line), name_rect);
            if focus == RenameAttrField::Name {
                let cx = text_input::input_cursor_x(name_rect, name_input);
                f.set_cursor_position((cx, name_inner.y));
            }
        }
        RenameAttrDialogState::Group { count, .. } => {
            f.render_widget(
                Paragraph::new(format!("{} files selected", count)).style(fill_style),
                Rect {
                    x: name_inner.x,
                    y: name_inner.y,
                    width: name_inner.width,
                    height: 1,
                },
            );
        }
    }

    let name_block_style = if focus == RenameAttrField::Name {
        focus_border
    } else {
        cyan
    };
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" File name ")
            .style(name_block_style),
        name_rect,
    );

    // 2. Permission (under File name, same column)
    let perm_block_h = 12 + 1 + 2; // 12 checkboxes + 1 octal line + border
    let perm_top = name_rect.y + name_rect.height + gap;
    let perm_rect = Rect {
        x: inner.x,
        y: perm_top,
        width: left_w,
        height: perm_block_h.min(inner.height.saturating_sub(perm_top - inner.y)),
    };
    let perm_inner = perm_rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    let (mode, perm_focus, _owner, _group, user_list, group_list, user_index, group_index) = match d
    {
        RenameAttrDialogState::Single {
            mode,
            perm_focus,
            owner,
            group,
            user_list,
            group_list,
            user_index,
            group_index,
            ..
        }
        | RenameAttrDialogState::Group {
            mode,
            perm_focus,
            owner,
            group,
            user_list,
            group_list,
            user_index,
            group_index,
            ..
        } => (
            mode,
            perm_focus,
            owner,
            group,
            user_list,
            group_list,
            user_index,
            group_index,
        ),
    };

    for (i, label) in PERM_LABELS.iter().enumerate() {
        let checked = mode_has_bit(*mode, PERM_BITS[i]);
        let mark = if checked { "[x]" } else { "[ ]" };
        let style = if i == *perm_focus {
            fill_style.bg(Color::Blue).fg(Color::White)
        } else {
            fill_style
        };
        f.render_widget(
            Paragraph::new(format!("{} {}", mark, label)).style(style),
            Rect {
                x: perm_inner.x,
                y: perm_inner.y + i as u16,
                width: perm_inner.width,
                height: 1,
            },
        );
    }
    let octal = format!("{:o}", *mode & 0o7777);
    f.render_widget(
        Paragraph::new(format!("Permissions (octal): {}", octal))
            .style(Style::default().fg(Color::DarkGray)),
        Rect {
            x: perm_inner.x,
            y: perm_inner.y + 12,
            width: perm_inner.width,
            height: 1,
        },
    );

    let perm_block_style = if focus == RenameAttrField::Permissions {
        focus_border
    } else {
        cyan
    };
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" Permission ")
            .style(perm_block_style),
        perm_rect,
    );

    // 3. Owner name (right top), 4. Group name (right bottom)
    let list_rect_h = (inner.height / 2).saturating_sub(1);
    let list_h = (list_rect_h.saturating_sub(2) as usize).max(1);
    let half_h = inner.height / 2;

    let user_rect = Rect {
        x: inner.x + left_w + gap,
        y: inner.y,
        width: right_w,
        height: list_rect_h,
    };
    let group_rect = Rect {
        x: inner.x + left_w + gap,
        y: inner.y + half_h,
        width: right_w,
        height: list_rect_h,
    };

    let ulen = user_list.len();
    let glen = group_list.len();
    let ui = (*user_index).min(ulen.saturating_sub(1).max(0));
    let gi = (*group_index).min(glen.saturating_sub(1).max(0));
    let max_start_u = ulen.saturating_sub(list_h).max(0);
    let max_start_g = glen.saturating_sub(list_h).max(0);
    let user_start = (ui + 1).saturating_sub(list_h).min(max_start_u);
    let group_start = (gi + 1).saturating_sub(list_h).min(max_start_g);
    let user_visible: Vec<_> = user_list
        .iter()
        .skip(user_start)
        .take(list_h)
        .enumerate()
        .map(|(i, u)| {
            let style = if user_start + i == ui {
                fill_style.bg(Color::Blue).fg(Color::White)
            } else {
                fill_style
            };
            ListItem::new(u.as_str()).style(style)
        })
        .collect();
    let group_visible: Vec<_> = group_list
        .iter()
        .skip(group_start)
        .take(list_h)
        .enumerate()
        .map(|(i, g)| {
            let style = if group_start + i == gi {
                fill_style.bg(Color::Blue).fg(Color::White)
            } else {
                fill_style
            };
            ListItem::new(g.as_str()).style(style)
        })
        .collect();

    let user_block_style = if focus == RenameAttrField::User {
        focus_border
    } else {
        cyan
    };
    let group_block_style = if focus == RenameAttrField::Group {
        focus_border
    } else {
        cyan
    };
    f.render_widget(
        List::new(user_visible).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Owner name ")
                .style(user_block_style),
        ),
        user_rect,
    );
    f.render_widget(
        List::new(group_visible).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Group name ")
                .style(group_block_style),
        ),
        group_rect,
    );

    let hint_rect = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(
            "Tab: switch area   Space: toggle perm   Enter: run   ↑↓: move   Esc: Cancel",
        )
        .style(Style::default().fg(Color::DarkGray)),
        hint_rect,
    );

    if let Some(msg) = app.rename_attr_error.as_ref() {
        let area = f.area();
        let aw = 52u16.min(area.width.saturating_sub(4));
        let ah = 6u16;
        let ax = area.x + (area.width.saturating_sub(aw)) / 2;
        let ay = area.y + (area.height.saturating_sub(ah)) / 2;
        let alert_rect = Rect {
            x: ax,
            y: ay,
            width: aw,
            height: ah,
        };
        let err_bg = Color::Rgb(60, 60, 60);
        let err_style = Style::default().bg(err_bg).fg(Color::White);
        let red = Style::default().fg(Color::Red);
        f.render_widget(Clear, alert_rect);
        f.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(" Error ")
                .style(red),
            alert_rect,
        );
        let inner = alert_rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let lines: Vec<_> = msg.lines().take(3).map(|s| s.to_string()).collect();
        let text = if lines.is_empty() {
            "Error".to_string()
        } else {
            lines.join("\n")
        };
        f.render_widget(
            Paragraph::new(text).style(err_style),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: inner.height.saturating_sub(1),
            },
        );
        f.render_widget(
            Paragraph::new("Press any key to close").style(Style::default().fg(Color::DarkGray)),
            Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            },
        );
    }
}
