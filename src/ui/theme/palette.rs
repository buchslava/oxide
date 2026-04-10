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

    /// Classic dual-pane blue canvas, light gray–white text, yellow accents (Norton-style).
    pub const COMMANDER: Self = Self {
        modal_dim_bg: Color::Rgb(0, 12, 40),
        modal_dim_fg: Color::Rgb(130, 160, 210),
        dialog_bg: Color::Rgb(0, 28, 72),
        dialog_bg_secondary: Color::Rgb(0, 34, 82),
        text: Color::Rgb(230, 235, 252),
        text_muted: Color::Rgb(165, 190, 228),
        border: Color::Rgb(110, 200, 255),
        accent: Color::Rgb(255, 220, 100),
        focus_bg: Color::Rgb(255, 235, 80),
        focus_fg: Color::Black,
        list_highlight_bg: Color::Rgb(0, 54, 118),
        list_highlight_fg: Color::Rgb(255, 255, 255),
        input_bg_focused: Color::Rgb(0, 42, 98),
        input_bg_unfocused: Color::Rgb(0, 36, 86),
        input_selection_bg: Color::Rgb(22, 78, 140),
        help_section_marker: Color::Rgb(120, 210, 255),
        help_heading: Color::Rgb(180, 230, 255),
        help_key: Color::Rgb(255, 225, 120),
        help_body: Color::Rgb(228, 234, 250),
        help_dim: Color::Rgb(155, 180, 220),
        rename_border: Color::Rgb(140, 220, 255),
        rename_border_active: Color::Rgb(255, 250, 140),
        error_fg: Color::Rgb(255, 120, 120),
    };

    /// CRT amber / orange phosphor: black canvas, amber text, solid orange selection (inverse video).
    pub const ORANGE_MONOCHROME: Self = Self {
        modal_dim_bg: Color::Rgb(0, 0, 0),
        modal_dim_fg: Color::Rgb(140, 95, 40),
        dialog_bg: Color::Rgb(10, 8, 4),
        dialog_bg_secondary: Color::Rgb(22, 16, 8),
        text: Color::Rgb(255, 176, 0),
        text_muted: Color::Rgb(200, 140, 55),
        border: Color::Rgb(255, 184, 30),
        accent: Color::Rgb(255, 204, 0),
        focus_bg: Color::Rgb(255, 204, 0),
        focus_fg: Color::Black,
        list_highlight_bg: Color::Rgb(255, 200, 40),
        list_highlight_fg: Color::Black,
        input_bg_focused: Color::Rgb(36, 24, 10),
        input_bg_unfocused: Color::Rgb(26, 18, 8),
        input_selection_bg: Color::Rgb(90, 55, 18),
        help_section_marker: Color::Rgb(230, 155, 40),
        help_heading: Color::Rgb(255, 200, 70),
        help_key: Color::Rgb(255, 214, 90),
        help_body: Color::Rgb(255, 185, 35),
        help_dim: Color::Rgb(170, 120, 45),
        rename_border: Color::Rgb(255, 190, 50),
        rename_border_active: Color::Rgb(255, 220, 100),
        error_fg: Color::Rgb(255, 95, 45),
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

    pub const COMMANDER: Self = Self {
        background: Color::Rgb(0, 18, 58),
        border: Color::Rgb(120, 210, 255),
        gauge: Color::Rgb(80, 190, 255),
        section_label: Color::Yellow,
        path_text: Color::Rgb(235, 240, 255),
        hint: Color::Rgb(120, 155, 200),
    };

    pub const ORANGE_MONOCHROME: Self = Self {
        background: Color::Rgb(0, 0, 0),
        border: Color::Rgb(255, 176, 0),
        gauge: Color::Rgb(255, 200, 50),
        section_label: Color::Rgb(255, 210, 80),
        path_text: Color::Rgb(255, 190, 60),
        hint: Color::Rgb(150, 105, 38),
    };
}

// --- Main panels, frame, command line, bottom bar, F10 menu ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromePalette {
    pub main_background: Color,
    pub panel_border_fg: Color,
    pub panel_border_bg: Color,
    /// Typed command text on the bottom line.
    pub command_line_fg: Color,
    /// Directory + sigil prefix when the command line has focus.
    pub command_prompt_active_fg: Color,
    /// Same prefix when focus is on the panels (or elsewhere).
    pub command_prompt_inactive_fg: Color,
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
        command_prompt_active_fg: Color::Green,
        command_prompt_inactive_fg: Color::DarkGray,
        menu_overlay_bg: Color::Rgb(60, 60, 60),
        menu_hotkey: Color::Rgb(255, 180, 80),
        menu_label: Color::Rgb(180, 180, 180),
        menu_unavailable: Color::DarkGray,
        bottom_bar_path: Color::Rgb(255, 180, 80),
        bottom_bar_size: Color::Rgb(170, 200, 220),
        bottom_bar_success: Color::Green,
        column_separator: Color::White,
    };

    pub const COMMANDER: Self = Self {
        main_background: Color::Rgb(0, 26, 72),
        panel_border_fg: Color::Rgb(200, 220, 255),
        panel_border_bg: Color::Rgb(0, 26, 72),
        command_line_fg: Color::Rgb(235, 240, 255),
        command_prompt_active_fg: Color::Rgb(255, 230, 100),
        command_prompt_inactive_fg: Color::Rgb(110, 145, 195),
        menu_overlay_bg: Color::Rgb(0, 18, 58),
        menu_hotkey: Color::Rgb(255, 220, 90),
        menu_label: Color::Rgb(200, 215, 245),
        menu_unavailable: Color::Rgb(90, 120, 165),
        bottom_bar_path: Color::Rgb(255, 220, 95),
        bottom_bar_size: Color::Rgb(165, 210, 255),
        bottom_bar_success: Color::Rgb(140, 255, 160),
        column_separator: Color::Rgb(190, 210, 255),
    };

    pub const ORANGE_MONOCHROME: Self = Self {
        main_background: Color::Rgb(0, 0, 0),
        panel_border_fg: Color::Rgb(255, 176, 0),
        panel_border_bg: Color::Rgb(0, 0, 0),
        command_line_fg: Color::Rgb(255, 188, 40),
        command_prompt_active_fg: Color::Rgb(255, 210, 90),
        command_prompt_inactive_fg: Color::Rgb(130, 88, 32),
        menu_overlay_bg: Color::Rgb(14, 10, 4),
        menu_hotkey: Color::Rgb(255, 204, 0),
        menu_label: Color::Rgb(220, 160, 65),
        menu_unavailable: Color::Rgb(85, 58, 22),
        bottom_bar_path: Color::Rgb(255, 204, 0),
        bottom_bar_size: Color::Rgb(205, 145, 55),
        bottom_bar_success: Color::Rgb(255, 225, 120),
        column_separator: Color::Rgb(255, 168, 25),
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
    /// Dotfiles (name starts with `.`, except `..`); only when row is not selected.
    pub hidden_fg: Color,
    /// `> ` and folder-diff `C ` / `S ` / `X ` in panel file lists.
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
        hidden_fg: Color::Rgb(88, 90, 96),
        marked_prefix: Color::Rgb(120, 175, 255),
    };

    pub const COMMANDER: Self = Self {
        selected_fg: Color::Black,
        selected_bg: Color::Rgb(255, 235, 80),
        directory_fg: Color::Rgb(150, 230, 255),
        executable_fg: Color::Rgb(130, 255, 160),
        zip_fg: Color::Rgb(200, 170, 255),
        symlink_fg: Color::Rgb(255, 160, 230),
        file_fg: Color::Rgb(235, 240, 255),
        hidden_fg: Color::Rgb(88, 90, 96),
        marked_prefix: Color::Rgb(255, 210, 90),
    };

    pub const ORANGE_MONOCHROME: Self = Self {
        selected_fg: Color::Black,
        selected_bg: Color::Rgb(255, 204, 0),
        directory_fg: Color::Rgb(255, 200, 95),
        executable_fg: Color::Rgb(255, 215, 120),
        zip_fg: Color::Rgb(225, 155, 55),
        symlink_fg: Color::Rgb(255, 165, 95),
        file_fg: Color::Rgb(255, 176, 0),
        hidden_fg: Color::Rgb(110, 62, 14),
        marked_prefix: Color::Rgb(255, 214, 100),
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

    pub const COMMANDER: Self = Self {
        background: Color::Rgb(0, 26, 72),
        text: Color::Rgb(235, 240, 255),
        header_path: Color::Rgb(150, 225, 255),
        muted: Color::Rgb(120, 155, 200),
        hex_cursor_bg: Color::Rgb(0, 40, 100),
        hex_cursor_fg: Color::Rgb(255, 255, 255),
    };

    pub const ORANGE_MONOCHROME: Self = Self {
        background: Color::Rgb(0, 0, 0),
        text: Color::Rgb(255, 180, 25),
        header_path: Color::Rgb(255, 205, 80),
        muted: Color::Rgb(145, 100, 38),
        hex_cursor_bg: Color::Rgb(255, 195, 45),
        hex_cursor_fg: Color::Black,
    };

    #[inline]
    pub fn hex_cursor_highlight_style(self) -> Style {
        Style::default()
            .bg(self.hex_cursor_bg)
            .fg(self.hex_cursor_fg)
    }
}

// --- Ctrl+D two-file diff (aligned lines, synchronized scroll) ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffViewerPalette {
    pub background: Color,
    pub text: Color,
    pub header_path: Color,
    pub muted: Color,
    pub column_border: Color,
    /// Row on the opposite pane when the other side has no line (typically same as [`Self::background`]).
    pub gap_bg: Color,
    /// Text color for gap cells (usually matches `gap_bg` when padding is spaces).
    pub gap_fg: Color,
    pub removed_bg: Color,
    pub removed_fg: Color,
    pub added_bg: Color,
    pub added_fg: Color,
    pub changed_old_bg: Color,
    pub changed_old_fg: Color,
    pub changed_new_bg: Color,
    pub changed_new_fg: Color,
    pub line_number_fg: Color,
    /// Intra-line removed chars (left pane, changed rows).
    pub char_removed_bg: Color,
    pub char_removed_fg: Color,
    /// Intra-line inserted chars (right pane, changed rows).
    pub char_added_bg: Color,
    pub char_added_fg: Color,
}

impl DiffViewerPalette {
    /// Side-by-side diff: dark canvas, **green** for modified lines (both panes), **light blue**
    /// for lines only on one side, empty padding matches background (reference-style alignment).
    pub const OXIDE: Self = Self {
        background: Color::Rgb(26, 26, 30),
        text: Color::Rgb(245, 245, 250),
        header_path: Color::Rgb(200, 220, 255),
        muted: Color::Rgb(130, 135, 150),
        column_border: Color::Rgb(75, 80, 95),
        // Same as background: opposite pane reads as empty space, not a dashed stripe.
        gap_bg: Color::Rgb(26, 26, 30),
        gap_fg: Color::Rgb(26, 26, 30),
        removed_bg: Color::Rgb(72, 108, 148),
        removed_fg: Color::Rgb(248, 250, 255),
        added_bg: Color::Rgb(72, 108, 148),
        added_fg: Color::Rgb(248, 250, 255),
        changed_old_bg: Color::Rgb(42, 120, 72),
        changed_old_fg: Color::Rgb(255, 255, 255),
        changed_new_bg: Color::Rgb(42, 120, 72),
        changed_new_fg: Color::Rgb(255, 255, 255),
        line_number_fg: Color::Rgb(150, 155, 170),
        char_removed_bg: Color::Rgb(32, 95, 58),
        char_removed_fg: Color::Rgb(255, 255, 255),
        char_added_bg: Color::Rgb(58, 150, 92),
        char_added_fg: Color::Rgb(255, 255, 255),
    };

    pub const COMMANDER: Self = Self {
        background: Color::Rgb(0, 26, 68),
        text: Color::Rgb(240, 245, 255),
        header_path: Color::Rgb(185, 215, 255),
        muted: Color::Rgb(125, 155, 200),
        column_border: Color::Rgb(42, 78, 128),
        gap_bg: Color::Rgb(0, 26, 68),
        gap_fg: Color::Rgb(0, 26, 68),
        removed_bg: Color::Rgb(45, 95, 155),
        removed_fg: Color::Rgb(248, 250, 255),
        added_bg: Color::Rgb(45, 95, 155),
        added_fg: Color::Rgb(248, 250, 255),
        changed_old_bg: Color::Rgb(35, 115, 85),
        changed_old_fg: Color::Rgb(255, 255, 255),
        changed_new_bg: Color::Rgb(35, 115, 85),
        changed_new_fg: Color::Rgb(255, 255, 255),
        line_number_fg: Color::Rgb(140, 170, 210),
        char_removed_bg: Color::Rgb(28, 95, 68),
        char_removed_fg: Color::Rgb(255, 255, 255),
        char_added_bg: Color::Rgb(50, 145, 100),
        char_added_fg: Color::Rgb(255, 255, 255),
    };

    pub const ORANGE_MONOCHROME: Self = Self {
        background: Color::Rgb(0, 0, 0),
        text: Color::Rgb(255, 210, 150),
        header_path: Color::Rgb(255, 195, 80),
        muted: Color::Rgb(155, 105, 42),
        column_border: Color::Rgb(95, 62, 22),
        gap_bg: Color::Rgb(0, 0, 0),
        gap_fg: Color::Rgb(0, 0, 0),
        removed_bg: Color::Rgb(95, 52, 14),
        removed_fg: Color::Rgb(255, 220, 170),
        added_bg: Color::Rgb(95, 52, 14),
        added_fg: Color::Rgb(255, 220, 170),
        changed_old_bg: Color::Rgb(75, 48, 12),
        changed_old_fg: Color::Rgb(255, 230, 190),
        changed_new_bg: Color::Rgb(75, 48, 12),
        changed_new_fg: Color::Rgb(255, 230, 190),
        line_number_fg: Color::Rgb(165, 115, 45),
        char_removed_bg: Color::Rgb(58, 35, 8),
        char_removed_fg: Color::Rgb(255, 210, 140),
        char_added_bg: Color::Rgb(110, 65, 18),
        char_added_fg: Color::Rgb(255, 235, 200),
    };
}

// --- Toasts (ratatui widget + crossterm main-buffer overlay) ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastPalette {
    pub background: Color,
    pub foreground: Color,
    /// Warnings and errors (e.g. editor limit, settings save failure).
    pub alert_background: Color,
    pub alert_foreground: Color,
}

impl ToastPalette {
    pub const OXIDE: Self = Self {
        background: Color::Rgb(60, 60, 60),
        foreground: Color::Rgb(140, 200, 140),
        alert_background: Color::Rgb(120, 28, 32),
        alert_foreground: Color::Rgb(255, 236, 236),
    };

    pub const COMMANDER: Self = Self {
        background: Color::Rgb(0, 38, 82),
        foreground: Color::Rgb(160, 240, 190),
        alert_background: Color::Rgb(130, 35, 45),
        alert_foreground: Color::Rgb(255, 238, 238),
    };

    pub const ORANGE_MONOCHROME: Self = Self {
        background: Color::Rgb(38, 24, 8),
        foreground: Color::Rgb(255, 200, 90),
        alert_background: Color::Rgb(110, 38, 14),
        alert_foreground: Color::Rgb(255, 225, 200),
    };

    #[inline]
    pub fn ratatui_style(self) -> Style {
        Style::default().bg(self.background).fg(self.foreground)
    }

    #[inline]
    pub fn ratatui_style_alert(self) -> Style {
        Style::default()
            .bg(self.alert_background)
            .fg(self.alert_foreground)
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
    pub diff_viewer: DiffViewerPalette,
    pub toast: ToastPalette,
}

impl UiPalette {
    pub const OXIDE: Self = Self {
        dialog: DialogPalette::OXIDE,
        progress: ProgressPalette::OXIDE,
        chrome: ChromePalette::OXIDE,
        panel_list: PanelListPalette::OXIDE,
        viewer: ViewerPalette::OXIDE,
        diff_viewer: DiffViewerPalette::OXIDE,
        toast: ToastPalette::OXIDE,
    };

    pub const COMMANDER: Self = Self {
        dialog: DialogPalette::COMMANDER,
        progress: ProgressPalette::COMMANDER,
        chrome: ChromePalette::COMMANDER,
        panel_list: PanelListPalette::COMMANDER,
        viewer: ViewerPalette::COMMANDER,
        diff_viewer: DiffViewerPalette::COMMANDER,
        toast: ToastPalette::COMMANDER,
    };

    pub const ORANGE_MONOCHROME: Self = Self {
        dialog: DialogPalette::ORANGE_MONOCHROME,
        progress: ProgressPalette::ORANGE_MONOCHROME,
        chrome: ChromePalette::ORANGE_MONOCHROME,
        panel_list: PanelListPalette::ORANGE_MONOCHROME,
        viewer: ViewerPalette::ORANGE_MONOCHROME,
        diff_viewer: DiffViewerPalette::ORANGE_MONOCHROME,
        toast: ToastPalette::ORANGE_MONOCHROME,
    };
}

/// Built-in theme identifiers. Add variants when you ship more presets; resolve with [`ThemeId::palette`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeId {
    #[default]
    Oxide,
    Commander,
    OrangeMonochrome,
}

impl ThemeId {
    /// Presets shown in F9 → Theme (order = cycle order when multiple exist).
    pub const ALL: &'static [ThemeId] = &[
        ThemeId::Oxide,
        ThemeId::Commander,
        ThemeId::OrangeMonochrome,
    ];

    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            ThemeId::Oxide => "oxide",
            ThemeId::Commander => "commander",
            ThemeId::OrangeMonochrome => "orangemonochrome",
        }
    }

    /// Value stored in `settings.json` under `theme`.
    #[must_use]
    pub fn from_slug(s: &str) -> Self {
        if s.eq_ignore_ascii_case(ThemeId::OrangeMonochrome.slug()) {
            ThemeId::OrangeMonochrome
        } else if s.eq_ignore_ascii_case(ThemeId::Commander.slug()) {
            ThemeId::Commander
        } else if s.eq_ignore_ascii_case(ThemeId::Oxide.slug()) {
            ThemeId::Oxide
        } else {
            ThemeId::Oxide
        }
    }

    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            ThemeId::Oxide => "Oxide (default)",
            ThemeId::Commander => "Commander",
            ThemeId::OrangeMonochrome => "Orange monochrome",
        }
    }

    #[must_use]
    pub fn palette(self) -> UiPalette {
        match self {
            ThemeId::Oxide => UiPalette::OXIDE,
            ThemeId::Commander => UiPalette::COMMANDER,
            ThemeId::OrangeMonochrome => UiPalette::ORANGE_MONOCHROME,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeId;

    #[test]
    fn from_slug_commander() {
        assert_eq!(ThemeId::from_slug("commander"), ThemeId::Commander);
        assert_eq!(ThemeId::from_slug("COMMANDER"), ThemeId::Commander);
    }

    #[test]
    fn from_slug_unknown_falls_back_to_oxide() {
        assert_eq!(ThemeId::from_slug("no-such-theme"), ThemeId::Oxide);
    }

    #[test]
    fn from_slug_orange_monochrome() {
        assert_eq!(
            ThemeId::from_slug("orangemonochrome"),
            ThemeId::OrangeMonochrome
        );
        assert_eq!(
            ThemeId::from_slug("ORANGEMONOCHROME"),
            ThemeId::OrangeMonochrome
        );
    }
}
