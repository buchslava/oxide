use std::io;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseEvent, MouseEventKind},
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
                            // Panel navigation only when panels are visible
                            app.active_panel_mut().move_up();
                        } else {
                            // Command line scrolling only when panels are hidden
                            app.terminal_scroll_up();
                        }
                    }
                    KeyCode::Down => {
                        if app.panels_visible() {
                            // Panel navigation only when panels are visible
                            app.active_panel_mut().move_down();
                        } else {
                            // Command line scrolling only when panels are hidden
                            app.terminal_scroll_down();
                        }
                    }
                    KeyCode::Left => {
                        if app.panels_visible() {
                            // Panel navigation only when panels are visible
                            let panel_height = size().map(|(_, h)| h as usize).unwrap_or(20) - 3;
                            app.active_panel_mut().smart_move_left(panel_height);
                        } else {
                            // Command line scrolling only when panels are hidden
                            app.terminal_scroll_up();
                        }
                    }
                    KeyCode::Right => {
                        if app.panels_visible() {
                            // Panel navigation only when panels are visible
                            let panel_height = size().map(|(_, h)| h as usize).unwrap_or(20) - 3;
                            app.active_panel_mut().smart_move_right(panel_height);
                        } else {
                            // Command line scrolling only when panels are hidden
                            app.terminal_scroll_down();
                        }
                    }
                    KeyCode::PageUp => {
                        // Command line scrolling - independent of panel visibility
                        let terminal_height = size().map(|(_, h)| h as usize).unwrap_or(20);
                        app.terminal_scroll_page_up(terminal_height);
                    }
                    KeyCode::PageDown => {
                        // Command line scrolling - independent of panel visibility
                        let terminal_height = size().map(|(_, h)| h as usize).unwrap_or(20);
                        app.terminal_scroll_page_down(terminal_height);
                    }
                    KeyCode::Enter => {
                        if app.panels_visible() {
                            // Panels visible - normal Enter behavior
                            if !app.command_input().is_empty() {
                                app.execute_command()?;
                            } else {
                                app.active_panel_mut().enter_directory()?;
                            }
                        } else {
                            // Panels hidden - just add empty line to terminal output
                            if !app.command_input().is_empty() {
                                // Execute the command first
                                app.execute_command()?;
                            } else {
                                // Empty command - just add empty line
                                app.add_output_line(String::new());
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            Self::handle_ctrl_key(app, c)?;
                        } else if c == '\t' {
                            // Tab behavior depends on panel visibility
                            if app.panels_visible() {
                                app.switch_panel()?;
                            } else {
                                // Panels hidden - Tab behaves as normal Tab (insert tab character)
                                app.add_command_char('\t');
                                app.reset_cursor();
                            }
                        } else {
                            app.add_command_char(c);
                            app.reset_cursor();
                        }
                    }
                    KeyCode::Tab => {
                        // Tab behavior depends on panel visibility
                        if app.panels_visible() {
                            app.switch_panel()?;
                        } else {
                            // Panels hidden - Tab behaves as normal Tab (insert tab character)
                            app.add_command_char('\t');
                            app.reset_cursor();
                        }
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
            } else if let Event::Mouse(mouse_event) = event::read()? {
                Self::handle_mouse_event(app, mouse_event)?;
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

    fn handle_mouse_event(app: &mut AppState, mouse_event: MouseEvent) -> io::Result<()> {
        match mouse_event.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                // Mouse wheel up - scroll content up (inverse scrolling)
                app.terminal_scroll_up();
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                // Mouse wheel down - scroll content down (inverse scrolling)
                app.terminal_scroll_down();
            }
            _ => {
                // Handle other mouse events if needed
            }
        }
        Ok(())
    }
}
