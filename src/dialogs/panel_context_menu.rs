//! Right-click popup menu over the panel view (same actions as the bottom menu bar).

use crate::app::events::AppAction;
use crate::app::mouse::menu_bar_action;
use crate::app::state::AppState;
use crate::ui::menu_bar_key;
use crate::ui::Renderer;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

const MENU_WIDTH: u16 = 22;

#[derive(Clone, Copy)]
struct Entry {
    label: &'static str,
    key: u16,
}

const ENTRIES: &[Entry] = &[
    Entry {
        label: "Actions",
        key: menu_bar_key::ACTIONS,
    },
    Entry {
        label: "File",
        key: menu_bar_key::FILE,
    },
    Entry {
        label: "View",
        key: menu_bar_key::VIEW,
    },
    Entry {
        label: "Edit",
        key: menu_bar_key::EDIT,
    },
    Entry {
        label: "Copy",
        key: menu_bar_key::COPY,
    },
    Entry {
        label: "Move",
        key: menu_bar_key::MOVE,
    },
    Entry {
        label: "Folder",
        key: menu_bar_key::FOLDER,
    },
    Entry {
        label: "Delete",
        key: menu_bar_key::DELETE,
    },
    Entry {
        label: "Settings",
        key: menu_bar_key::SETTINGS,
    },
];

pub struct PanelContextMenuState {
    pub rect: Rect,
    pub focus: usize,
}

pub fn open(
    app: &mut AppState,
    col: u16,
    row: u16,
    term_w: u16,
    term_h: u16,
) {
    let height = ENTRIES.len() as u16 + 2;
    let x = col.min(term_w.saturating_sub(MENU_WIDTH));
    let y = row.min(term_h.saturating_sub(height));
    app.panel_context_menu = Some(PanelContextMenuState {
        rect: Rect {
            x,
            y,
            width: MENU_WIDTH,
            height,
        },
        focus: first_available_index(app).unwrap_or(0),
    });
}

pub fn close(app: &mut AppState) {
    app.panel_context_menu = None;
}

fn first_available_index(app: &AppState) -> Option<usize> {
    ENTRIES
        .iter()
        .position(|e| Renderer::is_menu_action_available(app, e.key))
}

fn move_focus(
    app: &AppState,
    focus: usize,
    delta: isize,
) -> usize {
    let n = ENTRIES.len();
    if n == 0 {
        return 0;
    }
    let mut i = focus;
    for _ in 0..n {
        i = (i as isize + delta).rem_euclid(n as isize) as usize;
        if Renderer::is_menu_action_available(app, ENTRIES[i].key) {
            return i;
        }
    }
    focus
}

fn activate(
    app: &mut AppState,
    index: usize,
) -> Option<AppAction> {
    let entry = ENTRIES.get(index)?;
    if !Renderer::is_menu_action_available(app, entry.key) {
        return None;
    }
    close(app);
    menu_bar_action(app, entry.key)
}

pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    _modifiers: KeyModifiers,
) -> Option<AppAction> {
    if app.panel_context_menu.is_none() {
        return None;
    }
    match code {
        KeyCode::Esc => {
            close(app);
            Some(AppAction::Continue)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let focus = app.panel_context_menu.as_ref().map(|s| s.focus).unwrap_or(0);
            let new_focus = move_focus(app, focus, -1);
            if let Some(state) = app.panel_context_menu.as_mut() {
                state.focus = new_focus;
            }
            Some(AppAction::Continue)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let focus = app.panel_context_menu.as_ref().map(|s| s.focus).unwrap_or(0);
            let new_focus = move_focus(app, focus, 1);
            if let Some(state) = app.panel_context_menu.as_mut() {
                state.focus = new_focus;
            }
            Some(AppAction::Continue)
        }
        KeyCode::Home => {
            let new_focus = first_available_index(app).unwrap_or(0);
            if let Some(state) = app.panel_context_menu.as_mut() {
                state.focus = new_focus;
            }
            Some(AppAction::Continue)
        }
        KeyCode::End => {
            let new_focus = ENTRIES
                .iter()
                .rposition(|e| Renderer::is_menu_action_available(app, e.key))
                .unwrap_or_else(|| app.panel_context_menu.as_ref().map(|s| s.focus).unwrap_or(0));
            if let Some(state) = app.panel_context_menu.as_mut() {
                state.focus = new_focus;
            }
            Some(AppAction::Continue)
        }
        KeyCode::Enter => {
            let focus = app.panel_context_menu.as_ref().map(|s| s.focus).unwrap_or(0);
            activate(app, focus).or(Some(AppAction::Continue))
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let digit = c as u16 - '0' as u16;
            if digit == 0 {
                return Some(AppAction::Continue);
            }
            if let Some(idx) = ENTRIES.iter().position(|e| e.key == digit) {
                return activate(app, idx).or(Some(AppAction::Continue));
            }
            Some(AppAction::Continue)
        }
        _ => Some(AppAction::Continue),
    }
}

pub fn handle_mouse(
    app: &mut AppState,
    mouse_event: &MouseEvent,
) -> Option<AppAction> {
    let state = app.panel_context_menu.as_ref()?;
    let (col, row) = (mouse_event.column, mouse_event.row);
    match mouse_event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if col < state.rect.x
                || col >= state.rect.x + state.rect.width
                || row < state.rect.y
                || row >= state.rect.y + state.rect.height
            {
                close(app);
                return Some(AppAction::Continue);
            }
            let rel = (row - state.rect.y).saturating_sub(1) as usize;
            if rel < ENTRIES.len() {
                return activate(app, rel).or(Some(AppAction::Continue));
            }
            Some(AppAction::Continue)
        }
        MouseEventKind::Down(MouseButton::Right) => {
            close(app);
            Some(AppAction::Continue)
        }
        _ => Some(AppAction::Continue),
    }
}

pub fn draw(
    f: &mut Frame,
    app: &AppState,
) {
    let Some(state) = app.panel_context_menu.as_ref() else {
        return;
    };
    let d = &app.ui_palette.dialog;
    let fill = d.fill_style();
    let border = fill.fg(d.border);
    let highlight = d.list_highlight_style();
    let muted = fill.fg(d.text_muted);

    f.render_widget(Clear, state.rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Menu ")
        .style(border);
    f.render_widget(block, state.rect);

    let inner = state.rect.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    for (i, entry) in ENTRIES.iter().enumerate() {
        let available = Renderer::is_menu_action_available(app, entry.key);
        let style = if i == state.focus && available {
            highlight
        } else if available {
            fill
        } else {
            muted
        };
        let label = format!("{} {}", entry.key, entry.label);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(label, style))),
            Rect {
                x: inner.x,
                y: inner.y + i as u16,
                width: inner.width,
                height: 1,
            },
        );
    }
}
