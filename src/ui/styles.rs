use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::browser::diff_viewer::FolderDiffTag;
use crate::ui::theme::PanelListPalette;

/// Create a file line from a pre-formatted display string (e.g. truncated). Use for narrow columns.
pub(crate) fn create_file_line_from_display(
    display: &str,
    list: &PanelListPalette,
    is_dir: bool,
    is_symlink: bool,
    is_executable: bool,
    is_archive: bool,
    is_selected: bool,
    is_marked: bool,
    folder_diff: Option<FolderDiffTag>,
    is_hidden_dotfile: bool,
) -> Line<'static> {
    let prefix_style = || {
        if is_selected {
            Style::default().fg(list.marked_prefix).bg(list.selected_bg)
        } else {
            Style::default().fg(list.marked_prefix)
        }
    };
    let mut spans = Vec::new();
    if is_marked {
        spans.push(Span::styled("> ", prefix_style()));
    } else if let Some(tag) = folder_diff {
        let pfx = match tag {
            FolderDiffTag::ContentDiff => "C ",
            FolderDiffTag::SizeDiff => "S ",
            FolderDiffTag::AbsentOnOther => "X ",
        };
        spans.push(Span::styled(pfx, prefix_style()));
    }
    let style = if is_selected {
        if is_dir {
            Style::default()
                .fg(list.selected_fg)
                .bg(list.selected_bg)
                .add_modifier(Modifier::BOLD)
        } else if is_archive {
            Style::default().fg(list.selected_fg).bg(list.selected_bg)
        } else if is_symlink {
            Style::default().fg(list.selected_fg).bg(list.selected_bg)
        } else if is_executable {
            Style::default().fg(list.selected_fg).bg(list.selected_bg)
        } else {
            Style::default().fg(list.selected_fg).bg(list.selected_bg)
        }
    } else if is_hidden_dotfile {
        if is_dir {
            Style::default()
                .fg(list.hidden_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(list.hidden_fg)
        }
    } else if is_dir {
        Style::default()
            .fg(list.directory_fg)
            .add_modifier(Modifier::BOLD)
    } else if is_archive {
        Style::default().fg(list.zip_fg)
    } else if is_symlink {
        Style::default().fg(list.symlink_fg)
    } else if is_executable {
        Style::default().fg(list.executable_fg)
    } else {
        Style::default().fg(list.file_fg)
    };
    spans.push(Span::styled(display.to_string(), style));
    Line::from(spans)
}
