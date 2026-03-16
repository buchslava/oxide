use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// --- Dialog styling (single source of truth for modal dialogs) ---

/// Background for dialog panels (grey).
pub const DIALOG_BG: Color = Color::Rgb(60, 60, 60);
/// Background for focused text input inside a dialog.
pub const DIALOG_INPUT_BG_FOCUSED: Color = Color::Rgb(28, 34, 46);
/// Background for unfocused text input inside a dialog.
pub const DIALOG_INPUT_BG_UNFOCUSED: Color = Color::Rgb(38, 44, 56);
/// Accent for option numbers (e.g. "1. Skip") and highlighted keys (Y/N).
pub const DIALOG_ACCENT: Color = Color::Rgb(255, 180, 80);
/// Focus highlight (selected button or option row).
pub const DIALOG_FOCUS: Color = Color::Cyan;

/// Mark prefix shown for files marked for group operations (F12).
fn mark_prefix_span() -> Span<'static> {
    Span::styled("> ", Style::default().fg(Color::Yellow))
}

/// Create a file line from a pre-formatted display string (e.g. truncated). Use for narrow columns.
pub(crate) fn create_file_line_from_display(
    display: &str,
    is_dir: bool,
    is_symlink: bool,
    is_executable: bool,
    is_zip: bool,
    is_selected: bool,
    is_marked: bool,
) -> Line<'static> {
    let mut spans = Vec::new();
    if is_marked {
        spans.push(mark_prefix_span());
    }
    let style = if is_dir {
        if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        }
    } else if is_symlink {
        if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::Magenta)
        }
    } else if is_executable {
        if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::Green)
        }
    } else if is_zip {
        let zip_color = Color::Rgb(160, 120, 255);
        if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(zip_color)
        }
    } else {
        if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        }
    };
    spans.push(Span::styled(display.to_string(), style));
    Line::from(spans)
}
