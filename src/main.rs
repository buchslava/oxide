use std::io::{self, Write};
use ratatui::{backend::CrosstermBackend, Terminal};
use crossterm::{
    event::{self, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

mod app_state;
mod events;
mod file_ops;
mod panel;
mod styles;
mod subshell;
mod ui;

use app_state::AppState;
use events::{EventHandler, AppAction};
use panel::PanelOperations;
use ui::Renderer;

fn reset_terminal_character_set_and_modes<W: Write>(out: &mut W) -> io::Result<()> {
    // Defensive terminal reset after shell relay/commands:
    // - ESC ( B / ESC ) B: ASCII G0/G1 (undo DEC special graphics)
    // - SGR reset + ensure wrap is enabled
    out.write_all(b"\x1b(B\x1b)B\x1b[0m\x1b[?7h")?;
    out.flush()?;
    Ok(())
}

fn get_or_create_subshell<'a>(
    subshell: &'a mut Option<subshell::Subshell>,
    cwd: &str,
) -> io::Result<&'a subshell::Subshell> {
    if subshell.is_none() {
        *subshell = Some(subshell::Subshell::spawn(cwd)?);
    }
    Ok(subshell.as_ref().unwrap())
}

/// Escape path for shell (single-quote style so spaces/special chars are safe).
fn shell_escape_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\"'\"'"))
}

fn main() -> Result<(), io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = AppState::new()?;
    let mut subshell: Option<subshell::Subshell> = None;

    // Process any events from EnterAlternateScreen (never discard keys).
    if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
        terminal.show_cursor()?;
        return Ok(());
    }
    // Show first frame; brief delay then process queue so first keypress is handled, not discarded.
    terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
    std::thread::sleep(std::time::Duration::from_millis(50));
    if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
        terminal.show_cursor()?;
        return Ok(());
    }

    loop {
        terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;

        match EventHandler::handle_events(&mut app)? {
            AppAction::Quit => break,
            AppAction::Suspend => {
                // --- Ctrl+O: hand terminal to subshell. Use single-writer flow (REFERENCE.md "Solution: Second Ctrl+O uglification"): do NOT use backend for leave alternate; write everything to stdout.
                terminal.flush()?;
                let _ = terminal.backend_mut().flush();
                let _ = std::io::stdout().flush();
                let prepared = subshell::Subshell::prepare_for_relay();
                {
                    let mut stdout = std::io::stdout().lock();
                    let _ = subshell::Subshell::write_relay_reset_sequence(&mut stdout);
                    let _ = stdout.write_all(b"\r\n");
                    let _ = stdout.flush();
                }
                if let Ok(sub) = get_or_create_subshell(&mut subshell, app.get_current_dir()) {
                    let _ = sub.run_relay_until_ctrl_o(true, Some(prepared));
                } else {
                    eprintln!("Subshell error");
                }
                let _ = reset_terminal_character_set_and_modes(terminal.backend_mut());
                // 3) Return: enter alternate first (Ratatui), then drain input (MC tty_flush_input), then clear + redraw.
                execute!(
                    terminal.backend_mut(),
                    EnterAlternateScreen,
                    crossterm::cursor::Hide,
                    crossterm::event::EnableMouseCapture
                )?;
                if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                    break;
                }
                app.focus_panel();
                terminal.clear()?;
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                // Do NOT terminal.flush() after draw(): draw() already flushes; extra flush can paint black.
                if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                    break;
                }
            }
            AppAction::RunCommand(cmd) => {
                // Run command in the subshell (MC-style): output and prompt stay visible; Ctrl+O returns to panels. Same single-writer flow as Suspend.
                let left_selected = app.left_panel_mut().get_selected_file().map(|f| f.name.to_string());
                let right_selected = app.right_panel_mut().get_selected_file().map(|f| f.name.to_string());
                let cwd = app.get_current_dir().to_string();
                terminal.flush()?;
                let _ = terminal.backend_mut().flush();
                let _ = std::io::stdout().flush();
                let prepared = subshell::Subshell::prepare_for_relay();
                {
                    let mut stdout = std::io::stdout().lock();
                    let _ = subshell::Subshell::write_relay_reset_sequence(&mut stdout);
                    let _ = stdout.write_all(b"\r\n");
                    let _ = stdout.flush();
                }
                if let Ok(sub) = get_or_create_subshell(&mut subshell, app.get_current_dir()) {
                    let _ = sub.run_command_then_relay(&cwd, &cmd, Some(prepared));
                } else {
                    eprintln!("Subshell error");
                }
                let _ = reset_terminal_character_set_and_modes(terminal.backend_mut());
                // 3) Return: enter alternate, drain, focus panel, clear + draw (panels visible again).
                execute!(
                    terminal.backend_mut(),
                    EnterAlternateScreen,
                    crossterm::cursor::Hide,
                    crossterm::event::EnableMouseCapture
                )?;
                if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                    break;
                }
                app.focus_panel();
                terminal.clear()?;
                let _ = app.left_panel_mut().refresh_files_restore_selection(left_selected.as_deref());
                let _ = app.right_panel_mut().refresh_files_restore_selection(right_selected.as_deref());
                terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;
                if let Some(AppAction::Quit) = EventHandler::process_queued_events(&mut app)? {
                    break;
                }
            }
            AppAction::Continue => {}
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
