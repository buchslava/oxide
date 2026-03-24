use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::theme::PanelListPalette;

/// Create a file line from a pre-formatted display string (e.g. truncated). Use for narrow columns.
pub(crate) fn create_file_line_from_display(
    display: &str,
    list: &PanelListPalette,
    is_dir: bool,
    is_symlink: bool,
    is_executable: bool,
    is_zip: bool,
    is_selected: bool,
    is_marked: bool,
) -> Line<'static> {
    let mut spans = Vec::new();
    if is_marked {
        spans.push(Span::styled(
            "> ",
            Style::default().fg(list.marked_prefix),
        ));
    }
    let style = if is_dir {
        if is_selected {
            Style::default()
                .fg(list.selected_fg)
                .bg(list.selected_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(list.directory_fg)
                .add_modifier(Modifier::BOLD)
        }
    } else if is_symlink {
        if is_selected {
            Style::default()
                .fg(list.selected_fg)
                .bg(list.selected_bg)
        } else {
            Style::default().fg(list.symlink_fg)
        }
    } else if is_executable {
        if is_selected {
            Style::default()
                .fg(list.selected_fg)
                .bg(list.selected_bg)
        } else {
            Style::default().fg(list.executable_fg)
        }
    } else if is_zip {
        if is_selected {
            Style::default()
                .fg(list.selected_fg)
                .bg(list.selected_bg)
        } else {
            Style::default().fg(list.zip_fg)
        }
    } else if is_selected {
        Style::default()
            .fg(list.selected_fg)
            .bg(list.selected_bg)
    } else {
        Style::default().fg(list.file_fg)
    };
    spans.push(Span::styled(display.to_string(), style));
    Line::from(spans)
}
