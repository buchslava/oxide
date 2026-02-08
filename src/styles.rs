use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use crate::file_ops::FileInfo;

pub fn file_span(name: &str) -> Span<'static> {
    Span::styled(name.to_string(), Style::default().fg(Color::White))
}

pub fn folder_span(name: &str) -> Span<'static> {
    let folder_name = format!("{}/", name);
    Span::styled(folder_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
}

pub fn selected_file_span(name: &str) -> Span<'static> {
    Span::styled(name.to_string(), Style::default().fg(Color::White).add_modifier(Modifier::REVERSED))
}

pub fn selected_folder_span(name: &str) -> Span<'static> {
    let folder_name = format!("{}/", name);
    Span::styled(folder_name, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::REVERSED))
}

pub fn create_file_line(file: &FileInfo, is_selected: bool) -> Line<'static> {
    if file.is_dir {
        if is_selected {
            return Line::from(vec![selected_folder_span(&file.name)]);
        } else {
            return Line::from(vec![folder_span(&file.name)]);
        }
    }

    if is_selected {
        return Line::from(vec![selected_file_span(&file.name)]);
    }

    Line::from(vec![file_span(&file.name)])
}
