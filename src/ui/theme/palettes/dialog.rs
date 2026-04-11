//! Modal dialogs, inputs, help, rename chrome.

use ratatui::style::{Color, Modifier, Style};

/// Colors for modal dialogs, inputs inside them, and the modal dim scrim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogPalette {
    pub modal_dim_bg: Color,
    pub modal_dim_fg: Color,
    pub dialog_bg: Color,
    pub dialog_bg_secondary: Color,
    pub text: Color,
    pub text_muted: Color,
    pub border: Color,
    pub accent: Color,
    pub focus_bg: Color,
    pub focus_fg: Color,
    pub list_highlight_bg: Color,
    pub list_highlight_fg: Color,
    pub input_bg_focused: Color,
    pub input_bg_unfocused: Color,
    pub input_selection_bg: Color,
    pub help_section_marker: Color,
    pub help_heading: Color,
    pub help_key: Color,
    pub help_body: Color,
    pub help_dim: Color,
    pub rename_border: Color,
    pub rename_border_active: Color,
    pub error_fg: Color,
}

impl DialogPalette {
    #[inline]
    pub fn fill_style(self) -> Style {
        Style::default().bg(self.dialog_bg).fg(self.text)
    }

    #[inline]
    pub fn fill_secondary_style(self) -> Style {
        Style::default().bg(self.dialog_bg_secondary).fg(self.text)
    }

    #[inline]
    pub fn border_block_style(self) -> Style {
        Style::default()
            .bg(self.dialog_bg)
            .fg(self.border)
            .add_modifier(Modifier::BOLD)
    }

    #[inline]
    pub fn dim_layer_style(self) -> Style {
        Style::default().bg(self.modal_dim_bg).fg(self.modal_dim_fg)
    }

    #[inline]
    pub fn focus_row_style(self) -> Style {
        Style::default().bg(self.focus_bg).fg(self.focus_fg)
    }

    #[inline]
    pub fn list_highlight_style(self) -> Style {
        Style::default()
            .bg(self.list_highlight_bg)
            .fg(self.list_highlight_fg)
            .add_modifier(Modifier::BOLD)
    }
}
