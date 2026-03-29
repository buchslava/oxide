//! Semantic color groups for the whole TUI. Every [`ratatui::style::Color`] used in the app
//! should originate here (or from a theme that produces a [`UiPalette`]).

use crossterm::style::Color as CrosstermColor;
use ratatui::style::{Color, Modifier, Style};

// --- Dialogs & modals (F1, F2, F7, F9, Find, confirms, …) ---

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
    pub const OXIDE: Self = Self {
        modal_dim_bg: Color::Rgb(18, 18, 24),
        modal_dim_fg: Color::DarkGray,
        dialog_bg: Color::Rgb(32, 34, 40),
        dialog_bg_secondary: Color::Rgb(44, 46, 54),
        text: Color::Rgb(235, 238, 245),
        text_muted: Color::Rgb(165, 172, 185),
        border: Color::Rgb(130, 210, 255),
        accent: Color::Rgb(255, 180, 80),
        focus_bg: Color::Cyan,
        focus_fg: Color::Black,
        list_highlight_bg: Color::Rgb(0, 110, 150),
        list_highlight_fg: Color::Rgb(255, 255, 255),
        // Lighter than `dialog_bg` so the field reads as a distinct well (focused was darker before).
        input_bg_focused: Color::Rgb(58, 64, 82),
        input_bg_unfocused: Color::Rgb(48, 52, 66),
        input_selection_bg: Color::Rgb(72, 98, 140),
        help_section_marker: Color::Rgb(90, 170, 210),
        help_heading: Color::Rgb(150, 230, 255),
        help_key: Color::Rgb(255, 205, 120),
        help_body: Color::Rgb(235, 238, 245),
        help_dim: Color::Rgb(165, 172, 185),
        rename_border: Color::Rgb(115, 235, 255),
        rename_border_active: Color::Rgb(255, 255, 110),
        error_fg: Color::Rgb(255, 90, 90),
    };

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

// --- Copy / move / delete / archive progress overlays ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressPalette {
    pub background: Color,
    pub border: Color,
    pub gauge: Color,
    pub section_label: Color,
    pub path_text: Color,
    pub hint: Color,
}

impl ProgressPalette {
    pub const OXIDE: Self = Self {
        background: Color::Rgb(25, 40, 60),
        border: Color::Cyan,
        gauge: Color::Cyan,
        section_label: Color::Yellow,
        path_text: Color::White,
        hint: Color::DarkGray,
    };
}

// --- Main panels, frame, command line, bottom bar, F10 menu ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromePalette {
    pub main_background: Color,
    pub panel_border_fg: Color,
    pub panel_border_bg: Color,
    pub command_line_fg: Color,
    pub menu_overlay_bg: Color,
    pub menu_hotkey: Color,
    pub menu_label: Color,
    pub menu_unavailable: Color,
    pub bottom_bar_path: Color,
    pub bottom_bar_size: Color,
    pub bottom_bar_success: Color,
    pub column_separator: Color,
}

impl ChromePalette {
    pub const OXIDE: Self = Self {
        main_background: Color::Rgb(30, 30, 35),
        panel_border_fg: Color::White,
        panel_border_bg: Color::Rgb(30, 30, 35),
        command_line_fg: Color::White,
        menu_overlay_bg: Color::Rgb(60, 60, 60),
        menu_hotkey: Color::Rgb(255, 180, 80),
        menu_label: Color::Rgb(180, 180, 180),
        menu_unavailable: Color::DarkGray,
        bottom_bar_path: Color::Rgb(255, 180, 80),
        bottom_bar_size: Color::Rgb(170, 200, 220),
        bottom_bar_success: Color::Green,
        column_separator: Color::White,
    };
}

// --- File list (both panels) ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelListPalette {
    pub selected_fg: Color,
    pub selected_bg: Color,
    pub directory_fg: Color,
    pub executable_fg: Color,
    pub zip_fg: Color,
    pub symlink_fg: Color,
    pub file_fg: Color,
    pub marked_prefix: Color,
}

impl PanelListPalette {
    pub const OXIDE: Self = Self {
        selected_fg: Color::Black,
        selected_bg: Color::Cyan,
        directory_fg: Color::Cyan,
        executable_fg: Color::Green,
        zip_fg: Color::Rgb(160, 120, 255),
        symlink_fg: Color::Magenta,
        file_fg: Color::White,
        marked_prefix: Color::Yellow,
    };
}

// --- F3 viewer ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerPalette {
    pub background: Color,
    pub text: Color,
    pub header_path: Color,
    pub muted: Color,
    pub hex_cursor_bg: Color,
    pub hex_cursor_fg: Color,
}

impl ViewerPalette {
    pub const OXIDE: Self = Self {
        background: Color::Rgb(30, 30, 35),
        text: Color::White,
        header_path: Color::Cyan,
        muted: Color::DarkGray,
        hex_cursor_bg: Color::DarkGray,
        hex_cursor_fg: Color::White,
    };

    #[inline]
    pub fn hex_cursor_highlight_style(self) -> Style {
        Style::default()
            .bg(self.hex_cursor_bg)
            .fg(self.hex_cursor_fg)
    }
}

// --- Toasts (ratatui widget + crossterm main-buffer overlay) ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastPalette {
    pub background: Color,
    pub foreground: Color,
}

impl ToastPalette {
    pub const OXIDE: Self = Self {
        background: Color::Rgb(60, 60, 60),
        foreground: Color::Rgb(140, 200, 140),
    };

    #[inline]
    pub fn ratatui_style(self) -> Style {
        Style::default().bg(self.background).fg(self.foreground)
    }

    #[inline]
    pub fn crossterm_bg(self) -> CrosstermColor {
        rgb_to_crossterm(self.background)
    }

    #[inline]
    pub fn crossterm_fg(self) -> CrosstermColor {
        rgb_to_crossterm(self.foreground)
    }
}

fn rgb_to_crossterm(c: Color) -> CrosstermColor {
    match c {
        Color::Rgb(r, g, b) => CrosstermColor::Rgb { r, g, b },
        Color::Reset => CrosstermColor::Reset,
        _ => CrosstermColor::Reset,
    }
}

// --- Root palette ---

/// Full application palette: pass `&app.ui_palette` or store on [`crate::app::state::AppState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiPalette {
    pub dialog: DialogPalette,
    pub progress: ProgressPalette,
    pub chrome: ChromePalette,
    pub panel_list: PanelListPalette,
    pub viewer: ViewerPalette,
    pub toast: ToastPalette,
}

impl UiPalette {
    pub const OXIDE: Self = Self {
        dialog: DialogPalette::OXIDE,
        progress: ProgressPalette::OXIDE,
        chrome: ChromePalette::OXIDE,
        panel_list: PanelListPalette::OXIDE,
        viewer: ViewerPalette::OXIDE,
        toast: ToastPalette::OXIDE,
    };
}

/// Built-in theme identifiers. Add variants when you ship more presets; resolve with [`ThemeId::palette`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeId {
    #[default]
    Oxide,
}

impl ThemeId {
    #[must_use]
    pub fn palette(self) -> UiPalette {
        match self {
            ThemeId::Oxide => UiPalette::OXIDE,
        }
    }
}
