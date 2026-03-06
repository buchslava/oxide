use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Mark prefix shown for files marked for group operations (F12).
fn mark_prefix_span() -> Span<'static> {
    Span::styled("> ", Style::default().fg(Color::Yellow))
}

/// Create a file line from a pre-formatted display string (e.g. truncated). Use for narrow columns.
pub fn create_file_line_from_display(
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
