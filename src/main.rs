use std::io;
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use crossterm::{
    event::EnableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

mod app_state;
mod events;
mod file_ops;
mod panel;
mod styles;
mod ui;

use app_state::AppState;
use events::EventHandler;
use ui::Renderer;

fn main() -> Result<(), io::Error> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = AppState::new()?;

    // Main loop
    loop {
        // Update cursor for blinking effect
        app.update_cursor();
        
        // Draw UI
        terminal.draw(|f| Renderer::draw_ui(f, &mut app))?;

        // Handle events
        if EventHandler::handle_events(&mut app)? {
            break; // F10 was pressed
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
