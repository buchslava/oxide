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

/// MC-style: selected item = cyan background, black text.
pub fn selected_file_span(name: &str) -> Span<'static> {
    let display = format!(" {}", name);
    Span::styled(display, Style::default().fg(Color::Black).bg(Color::Cyan))
}

pub fn selected_folder_span(name: &str) -> Span<'static> {
    let folder_name = format!("/{}", name);
    Span::styled(folder_name, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD))
}

/// Executable file: leading "*", green as in screenshot (ls/MC style).
pub fn executable_span(name: &str) -> Span<'static> {
    let display = format!("*{}", name);
    Span::styled(display, Style::default().fg(Color::Green))
}

pub fn selected_executable_span(name: &str) -> Span<'static> {
    let display = format!("*{}", name);
    Span::styled(display, Style::default().fg(Color::Black).bg(Color::Cyan))
}

/// Symlink: leading "@" (MC style).
pub fn symlink_span(name: &str) -> Span<'static> {
    let display = format!("@{}", name);
    Span::styled(display, Style::default().fg(Color::Magenta))
}

pub fn selected_symlink_span(name: &str) -> Span<'static> {
    let display = format!("@{}", name);
    Span::styled(display, Style::default().fg(Color::Black).bg(Color::Cyan))
}

/// Mark prefix shown for files marked for group operations (F12).
fn mark_prefix_span() -> Span<'static> {
    Span::styled("> ", Style::default().fg(Color::Yellow))
}

pub fn create_file_line(file: &FileInfo, is_selected: bool, is_marked: bool) -> Line<'static> {
    let mut spans = Vec::new();
    if is_marked {
        spans.push(mark_prefix_span());
    }
    if file.is_dir {
        if is_selected {
            spans.push(selected_folder_span(&file.name));
        } else {
            spans.push(folder_span(&file.name));
        }
    } else if file.is_symlink {
        if is_selected {
            spans.push(selected_symlink_span(&file.name));
        } else {
            spans.push(symlink_span(&file.name));
        }
    } else if file.is_executable {
        if is_selected {
            spans.push(selected_executable_span(&file.name));
        } else {
            spans.push(executable_span(&file.name));
        }
    } else if is_selected {
        spans.push(selected_file_span(&file.name));
    } else {
        spans.push(file_span(&file.name));
    }
    Line::from(spans)
}

/// Create a file line from a pre-formatted display string (e.g. truncated). Use for narrow columns.
pub fn create_file_line_from_display(display: &str, is_dir: bool, is_symlink: bool, is_executable: bool, is_selected: bool, is_marked: bool) -> Line<'static> {
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
