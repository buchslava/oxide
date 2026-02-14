use std::io;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseEvent, MouseEventKind},
    terminal::size,
};
use crate::app_state::{AppState, Focus};
use crate::panel::PanelOperations;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    Continue,
    Quit,
    Suspend,
    RunCommand(String),
}

pub struct EventHandler;

impl EventHandler {
    /// Process all pending events: handle Key/Mouse (update app, return action), ignore FocusGained/Resize.
    /// Never discards keys. Returns the last action from a key/mouse, or None if queue empty or only non-keys.
    pub fn process_queued_events(app: &mut AppState) -> io::Result<Option<AppAction>> {
        let mut last_action = None;
        while event::poll(std::time::Duration::ZERO)? {
            let ev = event::read()?;
            if let Some(action) = Self::dispatch_event(app, ev)? {
                last_action = Some(action);
            }
        }
        Ok(last_action)
    }

    pub fn handle_events(app: &mut AppState) -> io::Result<AppAction> {
        // Process all queued events first (no block). Handle every Key/Mouse; drain non-keys.
        // This ensures rapid keypresses when switching panels (e.g. Tab then Down) are all applied.
        let mut last_action = None;
        while event::poll(std::time::Duration::ZERO)? {
            let ev = event::read()?;
            if let Some(action) = Self::dispatch_event(app, ev)? {
                last_action = Some(action);
            }
        }
        if last_action.is_some() {
            return Ok(last_action.unwrap());
        }
        // Queue empty: block for one event.
        if !event::poll(std::time::Duration::from_millis(100))? {
            return Ok(AppAction::Continue);
        }
        let ev = event::read()?;
        if let Some(action) = Self::dispatch_event(app, ev)? {
            return Ok(action);
        }
        Ok(AppAction::Continue)
    }

    /// Dispatch one event. Returns Some(action) if we handled a key and should return it, None to continue/drain.
    ///
    /// **Panel ↔ command line flow (MC-style):**
    /// - **Focus** is the single source of truth: `Panel` (default) or `CommandLine`.
    /// - **Panel → command line:** Type a printable character (focus moves and char is inserted), or press **F6** (focus only).
    /// - **Command line → panel:** **Tab** or **Esc** (focus returns to active panel; command line text is kept).
    /// - **Between panels:** **Tab** when focus is Panel switches left/right panel; from command line Tab first returns focus to panel.
    /// - All key handling branches on `app.focus` first; no key is handled by both panel and command line.
    fn dispatch_event(
        app: &mut AppState,
        ev: Event,
    ) -> io::Result<Option<AppAction>> {
        match ev {
            Event::Key(key) => {
                // Some terminals send Tab as Char('\t'); treat it as Tab.
                let code = match key.code {
                    KeyCode::Char('\t') => KeyCode::Tab,
                    other => other,
                };
                if app.focus == Focus::CommandLine {
                    return Ok(Some(Self::handle_command_line_key(app, code, key.modifiers)));
                }
                // Panel height must match draw (area.height - 2) so scroll stays in sync after panel switch / Ctrl+O.
                let panel_height = size()
                    .map(|(_, h)| h as usize)
                    .unwrap_or(24)
                    .saturating_sub(2)
                    .max(1);
                // Panel has focus: navigation, panel switch, or move to command line.
                match code {
                    KeyCode::Up => app.active_panel_mut().move_up(panel_height),
                    KeyCode::Down => app.active_panel_mut().move_down(panel_height),
                    KeyCode::Left => app.active_panel_mut().smart_move_left(panel_height),
                    KeyCode::Right => app.active_panel_mut().smart_move_right(panel_height),
                    KeyCode::PageUp => app.active_panel_mut().page_up(panel_height),
                    KeyCode::PageDown => app.active_panel_mut().page_down(panel_height),
                    KeyCode::Enter => app.active_panel_mut().enter_directory()?,
                    KeyCode::Char(c) => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            return Ok(Some(Self::handle_ctrl_key(app, c)));
                        }
                        if c.is_ascii() && !c.is_control() {
                            app.focus_command_line();
                            app.command_line_insert(c);
                        }
                    }
                    KeyCode::Tab => app.switch_panel()?,
                    KeyCode::F(6) => app.focus_command_line(), // MC: "Move" = focus command line
                    KeyCode::F(10) => return Ok(Some(AppAction::Quit)),
                    _ => {}
                }
                Ok(Some(AppAction::Continue))
            }
            Event::Mouse(mouse_event) => {
                Self::handle_mouse_event(app, mouse_event)?;
                Ok(Some(AppAction::Continue))
            }
            _ => Ok(None), // FocusGained, Resize, etc. - drain
        }
    }

    fn handle_command_line_key(
        app: &mut AppState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> AppAction {
        // Tab may be passed as KeyCode::Tab (normalized from Char('\t') in dispatch_event).
        match code {
            KeyCode::Char(c) => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    if c == 'o' {
                        return AppAction::Suspend;
                    }
                    if c == 'c' {
                        app.command_line_clear();
                        return AppAction::Continue;
                    }
                }
                app.command_line_insert(c);
                AppAction::Continue
            }
            KeyCode::Backspace => {
                app.command_line_backspace();
                AppAction::Continue
            }
            KeyCode::Left => {
                app.command_line_move_left();
                AppAction::Continue
            }
            KeyCode::Right => {
                app.command_line_move_right();
                AppAction::Continue
            }
            KeyCode::Enter => {
                let cmd = app.take_command_line();
                if !cmd.trim().is_empty() {
                    AppAction::RunCommand(cmd)
                } else {
                    AppAction::Continue
                }
            }
            KeyCode::Tab => {
                app.focus_panel();
                AppAction::Continue
            }
            KeyCode::Esc => {
                app.focus_panel();
                AppAction::Continue
            }
            _ => AppAction::Continue,
        }
    }

    fn handle_ctrl_key(app: &mut AppState, c: char) -> AppAction {
        match c {
            'o' => AppAction::Suspend,
            't' => {
                app.toggle_view_mode();
                AppAction::Continue
            }
            'r' => {
                let _ = app.active_panel_mut().refresh_files();
                AppAction::Continue
            }
            _ => AppAction::Continue,
        }
    }

    fn handle_mouse_event(app: &mut AppState, mouse_event: MouseEvent) -> io::Result<()> {
        let panel_height = size()
            .map(|(_, h)| h as usize)
            .unwrap_or(24)
            .saturating_sub(2)
            .max(1);
        match mouse_event.kind {
            MouseEventKind::ScrollUp => app.active_panel_mut().move_up(panel_height),
            MouseEventKind::ScrollDown => app.active_panel_mut().move_down(panel_height),
            _ => {}
        }
        Ok(())
    }
}
