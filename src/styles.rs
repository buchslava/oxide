use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use crate::file_ops::FileInfo;

/// Regular file: leading space as in ls -F style.
pub fn file_span(name: &str) -> Span<'static> {
    let display = format!(" {}", name);
    Span::styled(display, Style::default().fg(Color::White))
}

/// Folder: leading "/" as in screenshot.
pub fn folder_span(name: &str) -> Span<'static> {
    let folder_name = format!("/{}", name);
    Span::styled(folder_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
}

pub fn selected_file_span(name: &str) -> Span<'static> {
    let display = format!(" {}", name);
    Span::styled(display, Style::default().fg(Color::White).add_modifier(Modifier::REVERSED))
}

pub fn selected_folder_span(name: &str) -> Span<'static> {
    let folder_name = format!("/{}", name);
    Span::styled(folder_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::REVERSED))
}

/// Executable file: leading "*", green as in screenshot (ls/MC style).
pub fn executable_span(name: &str) -> Span<'static> {
    let display = format!("*{}", name);
    Span::styled(display, Style::default().fg(Color::Green))
}

pub fn selected_executable_span(name: &str) -> Span<'static> {
    let display = format!("*{}", name);
    Span::styled(display, Style::default().fg(Color::Green).add_modifier(Modifier::REVERSED))
}

pub fn create_file_line(file: &FileInfo, is_selected: bool) -> Line<'static> {
    if file.is_dir {
        if is_selected {
            return Line::from(vec![selected_folder_span(&file.name)]);
        } else {
            return Line::from(vec![folder_span(&file.name)]);
        }
    }

    if file.is_executable {
        if is_selected {
            return Line::from(vec![selected_executable_span(&file.name)]);
        }
        return Line::from(vec![executable_span(&file.name)]);
    }

    if is_selected {
        return Line::from(vec![selected_file_span(&file.name)]);
    }

    Line::from(vec![file_span(&file.name)])
}

/// Create a file line from a pre-formatted display string (e.g. truncated). Use for narrow columns.
pub fn create_file_line_from_display(display: &str, is_dir: bool, is_executable: bool, is_selected: bool) -> Line<'static> {
    let style = if is_dir {
        if is_selected {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        }
    } else if is_executable {
        if is_selected {
            Style::default().fg(Color::Green).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(Color::Green)
        }
    } else {
        if is_selected {
            Style::default().fg(Color::White).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(Color::White)
        }
    };
    Line::from(Span::styled(display.to_string(), style))
}
