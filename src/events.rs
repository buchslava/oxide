use std::io;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::size,
};
use crate::app_state::AppState;
use crate::panel::PanelOperations;

pub struct EventHandler;

impl EventHandler {
    pub fn handle_events(app: &mut AppState) -> io::Result<bool> {
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Up => {
                        if app.panels_visible() {
                            app.active_panel_mut().move_up();
                        } else {
                            app.terminal_scroll_up();
                        }
                    }
                    KeyCode::Down => {
                        if app.panels_visible() {
                            app.active_panel_mut().move_down();
                        } else {
                            app.terminal_scroll_down();
                        }
                    }
                    KeyCode::Left => {
                        if app.panels_visible() {
                            let panel_height = size().map(|(_, h)| h as usize).unwrap_or(20) - 3;
                            app.active_panel_mut().smart_move_left(panel_height);
                        } else {
                            let terminal_height = size().map(|(_, h)| h as usize).unwrap_or(20);
                            app.terminal_scroll_page_up(terminal_height);
                        }
                    }
                    KeyCode::Right => {
                        if app.panels_visible() {
                            let panel_height = size().map(|(_, h)| h as usize).unwrap_or(20) - 3;
                            app.active_panel_mut().smart_move_right(panel_height);
                        } else {
                            let terminal_height = size().map(|(_, h)| h as usize).unwrap_or(20);
                            app.terminal_scroll_page_down(terminal_height);
                        }
                    }
                    KeyCode::Enter => {
                        if !app.command_input().is_empty() {
                            app.execute_command()?;
                        } else {
                            app.active_panel_mut().enter_directory()?;
                        }
                    }
                    KeyCode::Char(c) => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            Self::handle_ctrl_key(app, c)?;
                        } else if c == '\t' {
                            app.switch_panel()?;
                        } else {
                            app.add_command_char(c);
                            app.reset_cursor();
                        }
                    }
                    KeyCode::Tab => {
                        app.switch_panel()?;
                    }
                    KeyCode::Backspace => {
                        app.remove_command_char();
                        app.reset_cursor();
                    }
                    KeyCode::F(10) => {
                        return Ok(true); // Signal to quit
                    }
                    _ => {}
                }
            }
        }
        Ok(false) // Continue running
    }

    fn handle_ctrl_key(app: &mut AppState, c: char) -> io::Result<()> {
        match c {
            'o' => {
                app.toggle_panels();
            }
            't' => {
                app.toggle_view_mode();
            }
            'r' => {
                let _ = app.active_panel_mut().refresh_files();
            }
            _ => {
                app.add_command_char(c);
                app.reset_cursor();
            }
        }
        Ok(())
    }
}
