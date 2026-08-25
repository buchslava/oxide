use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::browser::diff_viewer::FolderDiffTag;
use crate::core::file_ops::FileInfo;
use crate::ui::theme::PanelListPalette;

/// Visual flags for a panel list row (shared by one- and two-column views).
pub(crate) struct PanelRowAttrs {
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_executable: bool,
    pub is_archive: bool,
    pub is_selected: bool,
    pub is_marked: bool,
    pub folder_diff: Option<FolderDiffTag>,
    pub is_hidden_dotfile: bool,
}

impl PanelRowAttrs {
    pub(crate) fn from_file(
        file: &FileInfo,
        is_archive: bool,
        is_selected: bool,
        is_marked: bool,
        folder_diff: Option<FolderDiffTag>,
    ) -> Self {
        Self {
            is_dir: file.is_dir,
            is_symlink: file.is_symlink,
            is_executable: file.is_executable,
            is_archive,
            is_selected,
            is_marked,
            folder_diff,
            is_hidden_dotfile: file.is_hidden_dotfile(),
        }
    }
}

pub(crate) fn mark_prefix(attrs: &PanelRowAttrs) -> &'static str {
    if attrs.is_marked {
        "> "
    } else {
        match attrs.folder_diff {
            Some(FolderDiffTag::ContentDiff) => "C ",
            Some(FolderDiffTag::SizeDiff) => "S ",
            Some(FolderDiffTag::AbsentOnOther) => "X ",
            None => "",
        }
    }
}

/// Style for the file name segment in a panel row (permissions/size use neutral column styling).
pub(crate) fn panel_file_name_style(
    list: &PanelListPalette,
    attrs: &PanelRowAttrs,
) -> Style {
    if attrs.is_selected {
        list.selected_row_style()
    } else if attrs.is_hidden_dotfile {
        if attrs.is_dir {
            Style::default()
                .fg(list.hidden_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(list.hidden_fg)
        }
    } else if attrs.is_dir {
        Style::default()
            .fg(list.directory_fg)
            .add_modifier(Modifier::BOLD)
    } else if attrs.is_archive {
        Style::default().fg(list.zip_fg)
    } else if attrs.is_symlink {
        Style::default().fg(list.symlink_fg)
    } else if attrs.is_executable {
        Style::default().fg(list.executable_fg)
    } else {
        Style::default().fg(list.file_fg)
    }
}

/// One panel row from a pre-truncated display string (`*name`, `/dir`, …).
pub(crate) fn create_file_line_from_display(
    display: String,
    list: &PanelListPalette,
    attrs: &PanelRowAttrs,
) -> Line<'static> {
    let prefix_style = || {
        if attrs.is_selected {
            Style::default().fg(list.marked_prefix).bg(list.selected_bg)
        } else {
            Style::default().fg(list.marked_prefix)
        }
    };
    let mut spans = Vec::new();
    let prefix = mark_prefix(attrs);
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix, prefix_style()));
    }
    let style = panel_file_name_style(list, attrs);
    spans.push(Span::styled(display, style));
    Line::from(spans)
}
