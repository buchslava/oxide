//! F1 "Settings" dialog. Two-column: section list (General, Left panel, Right panel, Info, Help) and content. Esc or mouse click closes.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app_state::{AppState, SettingsDialogState, SETTINGS_SECTIONS};
use crate::events::{AppAction, SettingChange};
use crate::file_ops::SORT_MODES;

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
    let state = app.settings_dialog.as_mut()?;
    let p = app.persisted_settings.clone();
    match code {
        KeyCode::Esc => {
            close(app);
            return Some(AppAction::SettingsClose);
        }
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) && c == 'o' => {
            return Some(AppAction::Suspend);
        }
        KeyCode::Left => {
            // Switch to left column (section list)
            if !state.focus_left {
                state.focus_left = true;
                state.content_focus = 0;
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Right => {
            // Switch to right column (options)
            if state.focus_left {
                state.focus_left = false;
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Tab => {
            if state.focus_left {
                state.focus_left = false;
            } else if state.selected_section == 1 || state.selected_section == 2 {
                // Cycle between widgets: View (0), Sort (1), Folders first (2), Show hidden (3)
                state.content_focus = (state.content_focus + 1) % 4;
            }
            return Some(AppAction::Continue);
        }
        KeyCode::BackTab => {
            if !state.focus_left {
                state.focus_left = true;
                state.content_focus = 0;
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Up => {
            if state.focus_left {
                let n = SETTINGS_SECTIONS.len();
                state.selected_section = (state.selected_section + n - 1) % n;
                return Some(AppAction::Continue);
            }
            // Right column: Up = previous item or move focus up
            match state.selected_section {
                1 => {
                    if state.content_focus == 1 {
                        return Some(AppAction::SettingChange(SettingChange::LeftSortCyclePrev));
                    }
                    if state.content_focus >= 2 {
                        state.content_focus -= 1;
                    } else {
                        return Some(AppAction::SettingChange(SettingChange::LeftViewCycle));
                    }
                }
                2 => {
                    if state.content_focus == 1 {
                        return Some(AppAction::SettingChange(SettingChange::RightSortCyclePrev));
                    }
                    if state.content_focus >= 2 {
                        state.content_focus -= 1;
                    } else {
                        return Some(AppAction::SettingChange(SettingChange::RightViewCycle));
                    }
                }
                _ => {}
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Down => {
            if state.focus_left {
                let n = SETTINGS_SECTIONS.len();
                state.selected_section = (state.selected_section + 1) % n;
                return Some(AppAction::Continue);
            }
            // Right column: Down = next item or move focus down
            match state.selected_section {
                1 => {
                    if state.content_focus == 0 {
                        if p.left_view.as_str() == "two" {
                            return Some(AppAction::SettingChange(SettingChange::LeftViewCycle));
                        }
                        state.content_focus = 1;
                    } else if state.content_focus == 1 {
                        return Some(AppAction::SettingChange(SettingChange::LeftSortCycle));
                    } else if state.content_focus < 3 {
                        state.content_focus += 1;
                    } else {
                        state.content_focus = 0;
                    }
                }
                2 => {
                    if state.content_focus == 0 {
                        if p.right_view.as_str() == "two" {
                            return Some(AppAction::SettingChange(SettingChange::RightViewCycle));
                        }
                        state.content_focus = 1;
                    } else if state.content_focus == 1 {
                        return Some(AppAction::SettingChange(SettingChange::RightSortCycle));
                    } else if state.content_focus < 3 {
                        state.content_focus += 1;
                    } else {
                        state.content_focus = 0;
                    }
                }
                _ => {}
            }
            return Some(AppAction::Continue);
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if state.focus_left {
                state.focus_left = false;
                return Some(AppAction::Continue);
            }
            let action = match state.selected_section {
                0 => Some(AppAction::SettingChange(SettingChange::AutosaveToggle)),
                1 => match state.content_focus {
                    0 => Some(AppAction::SettingChange(SettingChange::LeftViewCycle)),
                    1 => Some(AppAction::SettingChange(SettingChange::LeftSortCycle)),
                    2 => Some(AppAction::SettingChange(SettingChange::LeftDirsFirstToggle)),
                    _ => Some(AppAction::SettingChange(SettingChange::LeftShowHiddenToggle)),
                },
                2 => match state.content_focus {
                    0 => Some(AppAction::SettingChange(SettingChange::RightViewCycle)),
                    1 => Some(AppAction::SettingChange(SettingChange::RightSortCycle)),
                    2 => Some(AppAction::SettingChange(SettingChange::RightDirsFirstToggle)),
                    _ => Some(AppAction::SettingChange(SettingChange::RightShowHiddenToggle)),
                },
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

/// Help text lines for the Help section.
fn help_lines() -> Vec<Line<'static>> {
    let cyan = Color::Cyan;
    vec![
        Line::from(vec![Span::styled("Panels", cyan), Span::raw(":")]),
        Line::from(vec![Span::raw("  ↑↓ PgUp/PgDn  Navigate   Tab  Switch panel")]),
        Line::from(vec![Span::raw("  ← →          Move col   Enter  Open dir / run")]),
        Line::from(vec![Span::raw("  Space  Mark   *  Invert selection")]),
        Line::from(""),
        Line::from(vec![Span::styled("F-keys", cyan), Span::raw(": F1 Settings  F2 Rename  F3 View  F4 Edit")]),
        Line::from(vec![Span::raw("  F5 Copy  F6 Move  F7 New dir  F8 Delete  F10 Quit")]),
        Line::from(vec![Span::raw("  Ctrl+Q  Left panel settings   Ctrl+W  Right panel settings")]),
        Line::from(""),
        Line::from(vec![Span::styled("Shortcuts", cyan), Span::raw(": Ctrl+O  Shell   Ctrl+H  Hidden   Ctrl+G  Size   Ctrl+R  Refresh   Ctrl+T  View")]),
        Line::from(vec![Span::raw("  Type char → command line   Tab/Esc → panel")]),
        Line::from(""),
        Line::from(vec![Span::styled("Find file", cyan), Span::raw(" (Ctrl+F): Start dir, file pattern (*?), content pattern")]),
        Line::from(vec![Span::raw("  Tab/↑↓  move   Enter  start search or Chdir on result   Esc  stop search or close")]),
        Line::from(vec![Span::raw("  F3  View   F4  Edit  on selected result (dialog stays open)")]),
        Line::from(""),
        Line::from(vec![Span::styled("Editor", cyan), Span::raw(" (F4): Shift+←→↑↓ select  F3 lines  Ctrl+C/V  F2 Save  Esc exit")]),
        Line::from(vec![Span::raw("  Ctrl+F  Find in file")]),
        Line::from(""),
        Line::from(vec![Span::styled("Viewer", cyan), Span::raw(" (F3): Esc close  H  hex/text  ↑↓ PgUp/PgDn  scroll")]),
        Line::from(""),
        Line::from(vec![Span::styled("Create dir", cyan), Span::raw(" (F7): Tab  textarea↔buttons  Enter/Esc")]),
        Line::from(vec![Span::styled("Dialogs", cyan), Span::raw(": Tab/↑↓  choose   Enter  confirm   Esc  cancel")]),
    ]
}

/// Draw the Settings dialog: two-column layout.
pub fn draw(f: &mut Frame, app: &mut AppState) {
    let state = match &app.settings_dialog {
        Some(s) => s,
        None => return,
    };

    let area = f.area();
    const MIN_W: u16 = 72;
    const MIN_H: u16 = 28;
    let w = MIN_W.min(area.width.saturating_sub(4));
    let h = MIN_H.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };

    let grey_bg = Color::Rgb(60, 60, 60);
    let right_bg = Color::Rgb(50, 52, 58); // slightly darker tint for column 2
    let fill_style = Style::default().bg(grey_bg).fg(Color::White);
    let right_fill_style = Style::default().bg(right_bg).fg(Color::White);
    let border_style = fill_style.fg(Color::Cyan);
    let highlight_style = Style::default().bg(Color::Blue).fg(Color::White).add_modifier(Modifier::BOLD);

    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Settings / Help ")
        .style(border_style);
    f.render_widget(block, rect);

    let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
    let hint_h = 1u16;
    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(hint_h),
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(20)])
        .split(content_rect);

    let left_area = chunks[0];
    let right_area = chunks[1];

    // Left column: section list
    let list_items: Vec<ListItem> = SETTINGS_SECTIONS
        .iter()
        .map(|s| ListItem::new(*s).style(fill_style))
        .collect();
    let list = List::new(list_items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(highlight_style)
        .highlight_symbol("▸ ");
    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_section));
    f.render_stateful_widget(list, left_area, &mut list_state);

    // Right column: different background, then content by section
    let right_inner = right_area.inner(Margin { horizontal: 1, vertical: 1 });
    let fill_right = Paragraph::new(
        std::iter::repeat(Line::from(Span::raw(" ".repeat(right_area.width as usize))))
            .take(right_area.height as usize)
            .collect::<Vec<_>>(),
    )
    .style(right_fill_style);
    f.render_widget(fill_right, right_area);

    let p = &app.persisted_settings;
    let left_view_index = if p.left_view.as_str() == "one" { 1 } else { 0 };
    let right_view_index = if p.right_view.as_str() == "one" { 1 } else { 0 };
    let left_sort_index = SORT_MODES.iter().position(|s| *s == p.left_sort.as_str()).unwrap_or(0);
    let right_sort_index = SORT_MODES.iter().position(|s| *s == p.right_sort.as_str()).unwrap_or(0);
    match state.selected_section {
        0 => {
            let checkbox = if p.autosave { "[x]" } else { "[ ]" };
            let line = Line::from(vec![
                Span::raw(checkbox),
                Span::raw(" Autosave latest state"),
            ]);
            let para = Paragraph::new(line).style(right_fill_style);
            f.render_widget(para, right_inner);
        }
        1 => draw_panel_section(f, right_inner, right_fill_style, left_view_index, left_sort_index, p.left_dirs_first, p.left_show_hidden, state.content_focus),
        2 => draw_panel_section(f, right_inner, right_fill_style, right_view_index, right_sort_index, p.right_dirs_first, p.right_show_hidden, state.content_focus),
        3 => {
            let para = Paragraph::new(info_lines())
                .style(right_fill_style)
                .wrap(Wrap { trim: true });
            f.render_widget(para, right_inner);
        }
        4 => {
            let para = Paragraph::new(help_lines())
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
        Paragraph::new("↑↓ List  ← → Column  Tab  Options  Space/Enter  Toggle  Esc  Close")
            .style(fill_style.fg(Color::DarkGray))
            .alignment(Alignment::Center),
        hint_rect,
    );
}

/// Draw panel options (View, Sort, Folders first, Show hidden). Used by F1 Settings and by panel overlay.
pub(crate) fn draw_panel_section(
    f: &mut Frame,
    area: Rect,
    fill_style: Style,
    view_index: usize,
    sort_index: usize,
    dirs_first: bool,
    show_hidden: bool,
    content_focus: usize,
) {
    let view_highlight = Style::default().bg(Color::Blue).fg(Color::White);
    let mut y = area.y;
    let line_h = 1u16;

    // View: listbox
    let view_label = Line::from(Span::raw("View:"));
    f.render_widget(Paragraph::new(view_label).style(fill_style), Rect { x: area.x, y, width: area.width, height: line_h });
    y += line_h;

    for (i, opt) in VIEW_OPTS.iter().enumerate() {
        let (sym, style) = if i == view_index {
            ("◉ ", if content_focus == 0 { view_highlight } else { fill_style })
        } else {
            ("○ ", fill_style)
        };
        let line = Line::from(vec![Span::raw(sym), Span::raw(*opt)]);
        f.render_widget(Paragraph::new(line).style(style), Rect { x: area.x, y, width: area.width, height: line_h });
        y += line_h;
    }

    y += 1;

    // Sort: listbox
    let sort_label = Line::from(Span::raw("Sort:"));
    f.render_widget(Paragraph::new(sort_label).style(fill_style), Rect { x: area.x, y, width: area.width, height: line_h });
    y += line_h;

    for (i, opt) in SORT_OPTIONS.iter().enumerate() {
        let (sym, style) = if i == sort_index {
            ("◉ ", if content_focus == 1 { view_highlight } else { fill_style })
        } else {
            ("○ ", fill_style)
        };
        let line = Line::from(vec![Span::raw(sym), Span::raw(*opt)]);
        f.render_widget(Paragraph::new(line).style(style), Rect { x: area.x, y, width: area.width, height: line_h });
        y += line_h;
    }

    y += 1;

    // Folders first (directories listed before files)
    let dirs_first_chk = if dirs_first { "[x]" } else { "[ ]" };
    let dirs_first_style = if content_focus == 2 { view_highlight } else { fill_style };
    let dirs_first_line = Line::from(vec![Span::raw(dirs_first_chk), Span::raw(" Folders first")]);
    f.render_widget(Paragraph::new(dirs_first_line).style(dirs_first_style), Rect { x: area.x, y, width: area.width, height: line_h });
    y += line_h;

    let checkbox = if show_hidden { "[x]" } else { "[ ]" };
    let chk_style = if content_focus == 3 { view_highlight } else { fill_style };
    let chk_line = Line::from(vec![Span::raw(checkbox), Span::raw(" Show hidden files")]);
    f.render_widget(Paragraph::new(chk_line).style(chk_style), Rect { x: area.x, y, width: area.width, height: line_h });
}
