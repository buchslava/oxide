//! Terminal color depth: downgrade theme [`Color::Rgb`] for legacy emulators that mishandle 24-bit
//! sequences (see ratatui docs on Crossterm + truecolor).
//!
//! Override auto-detection with **`OXIDE_COLOR_DEPTH`**: `auto` (default), `truecolor`, `256`,
//! `16`.

use std::env;

use crossterm::style::Color as CrosstermColor;
use ratatui::style::Color;

use crate::ui::theme::palettes::{
    ChromePalette, DialogPalette, DiffViewerPalette, PanelListPalette, ProgressPalette,
    ToastPalette, ViewerPalette,
};
use crate::ui::theme::UiPalette;

/// How aggressively we remap [`Color::Rgb`] before drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Extended256,
    Basic16,
}

impl ColorDepth {
    /// Read [`ColorDepth`] from the environment (`OXIDE_COLOR_DEPTH` + `COLORTERM` / `TERM`).
    pub fn from_env() -> Self {
        match env::var("OXIDE_COLOR_DEPTH") {
            Ok(s) => match s.to_ascii_lowercase().trim() {
                "" | "auto" => Self::from_env_auto(),
                "truecolor" | "24bit" | "24-bit" | "rgb" => Self::TrueColor,
                "256" | "8bit" | "8-bit" => Self::Extended256,
                "16" | "ansi" | "4bit" | "4-bit" => Self::Basic16,
                _ => Self::from_env_auto(),
            },
            Err(_) => Self::from_env_auto(),
        }
    }

    fn from_env_auto() -> Self {
        if env::var("COLORTERM")
            .ok()
            .is_some_and(|v| v.eq_ignore_ascii_case("truecolor"))
        {
            return Self::TrueColor;
        }
        let term = env::var("TERM").unwrap_or_default();
        if term.contains("256color") || term.contains("256-color") {
            return Self::Extended256;
        }
        if term.contains("rxvt-unicode") || term.contains("kitty") || term.contains("alacritty") {
            return Self::Extended256;
        }
        Self::Basic16
    }

    /// Remap a single color for this depth (pass-through for non-RGB where possible).
    pub fn adapt_color(self, c: Color) -> Color {
        match self {
            Self::TrueColor => c,
            Self::Extended256 => match c {
                Color::Rgb(r, g, b) => Color::Indexed(rgb_to_ansi256(r, g, b)),
                Color::Indexed(i) => Color::Indexed(i.min(255)),
                _ => {
                    let (r, g, b) = ratatui_named_to_rgb(c);
                    Color::Indexed(rgb_to_ansi256(r, g, b))
                }
            },
            Self::Basic16 => match c {
                Color::Rgb(r, g, b) => Color::Indexed(rgb_to_ansi16_index(r, g, b)),
                Color::Indexed(i) if i <= 15 => Color::Indexed(i),
                Color::Indexed(i) => {
                    let (r, g, b) = ansi256_to_rgb(i);
                    Color::Indexed(rgb_to_ansi16_index(r, g, b))
                }
                _ => {
                    let (r, g, b) = ratatui_named_to_rgb(c);
                    Color::Indexed(rgb_to_ansi16_index(r, g, b))
                }
            },
        }
    }
}

/// Crossterm colors for overlays (main buffer): prefer ANSI values when the palette was downgraded.
pub fn ratatui_to_crossterm(c: Color) -> CrosstermColor {
    match c {
        Color::Reset => CrosstermColor::Reset,
        Color::Rgb(r, g, b) => CrosstermColor::Rgb { r, g, b },
        Color::Indexed(i) => CrosstermColor::AnsiValue(i.min(255)),
        Color::Black => CrosstermColor::Black,
        Color::Red => CrosstermColor::DarkRed,
        Color::Green => CrosstermColor::DarkGreen,
        Color::Yellow => CrosstermColor::DarkYellow,
        Color::Blue => CrosstermColor::DarkBlue,
        Color::Magenta => CrosstermColor::DarkMagenta,
        Color::Cyan => CrosstermColor::DarkCyan,
        Color::Gray => CrosstermColor::Grey,
        Color::DarkGray => CrosstermColor::DarkGrey,
        Color::LightRed => CrosstermColor::Red,
        Color::LightGreen => CrosstermColor::Green,
        Color::LightYellow => CrosstermColor::Yellow,
        Color::LightBlue => CrosstermColor::Blue,
        Color::LightMagenta => CrosstermColor::Magenta,
        Color::LightCyan => CrosstermColor::Cyan,
        Color::White => CrosstermColor::White,
    }
}

pub fn adapt_ui_palette(
    p: UiPalette,
    depth: ColorDepth,
) -> UiPalette {
    if depth == ColorDepth::TrueColor {
        return p;
    }
    let a = |c: Color| depth.adapt_color(c);
    UiPalette {
        dialog: DialogPalette {
            modal_dim_bg: a(p.dialog.modal_dim_bg),
            modal_dim_fg: a(p.dialog.modal_dim_fg),
            dialog_bg: a(p.dialog.dialog_bg),
            dialog_bg_secondary: a(p.dialog.dialog_bg_secondary),
            text: a(p.dialog.text),
            text_muted: a(p.dialog.text_muted),
            border: a(p.dialog.border),
            accent: a(p.dialog.accent),
            focus_bg: a(p.dialog.focus_bg),
            focus_fg: a(p.dialog.focus_fg),
            list_highlight_bg: a(p.dialog.list_highlight_bg),
            list_highlight_fg: a(p.dialog.list_highlight_fg),
            input_bg_focused: a(p.dialog.input_bg_focused),
            input_bg_unfocused: a(p.dialog.input_bg_unfocused),
            input_selection_bg: a(p.dialog.input_selection_bg),
            help_section_marker: a(p.dialog.help_section_marker),
            help_heading: a(p.dialog.help_heading),
            help_key: a(p.dialog.help_key),
            help_body: a(p.dialog.help_body),
            help_dim: a(p.dialog.help_dim),
            rename_border: a(p.dialog.rename_border),
            rename_border_active: a(p.dialog.rename_border_active),
            error_fg: a(p.dialog.error_fg),
        },
        progress: ProgressPalette {
            background: a(p.progress.background),
            border: a(p.progress.border),
            gauge: a(p.progress.gauge),
            section_label: a(p.progress.section_label),
            path_text: a(p.progress.path_text),
            hint: a(p.progress.hint),
        },
        chrome: ChromePalette {
            main_background: a(p.chrome.main_background),
            panel_border_fg: a(p.chrome.panel_border_fg),
            panel_border_bg: a(p.chrome.panel_border_bg),
            command_line_fg: a(p.chrome.command_line_fg),
            command_prompt_active_fg: a(p.chrome.command_prompt_active_fg),
            command_prompt_inactive_fg: a(p.chrome.command_prompt_inactive_fg),
            menu_overlay_bg: a(p.chrome.menu_overlay_bg),
            menu_hotkey: a(p.chrome.menu_hotkey),
            menu_label: a(p.chrome.menu_label),
            menu_unavailable: a(p.chrome.menu_unavailable),
            bottom_bar_path: a(p.chrome.bottom_bar_path),
            bottom_bar_size: a(p.chrome.bottom_bar_size),
            bottom_bar_success: a(p.chrome.bottom_bar_success),
            column_separator: a(p.chrome.column_separator),
        },
        panel_list: PanelListPalette {
            selected_fg: a(p.panel_list.selected_fg),
            selected_bg: a(p.panel_list.selected_bg),
            directory_fg: a(p.panel_list.directory_fg),
            executable_fg: a(p.panel_list.executable_fg),
            zip_fg: a(p.panel_list.zip_fg),
            symlink_fg: a(p.panel_list.symlink_fg),
            file_fg: a(p.panel_list.file_fg),
            hidden_fg: a(p.panel_list.hidden_fg),
            marked_prefix: a(p.panel_list.marked_prefix),
        },
        viewer: ViewerPalette {
            background: a(p.viewer.background),
            text: a(p.viewer.text),
            header_path: a(p.viewer.header_path),
            muted: a(p.viewer.muted),
            hex_cursor_bg: a(p.viewer.hex_cursor_bg),
            hex_cursor_fg: a(p.viewer.hex_cursor_fg),
        },
        diff_viewer: DiffViewerPalette {
            background: a(p.diff_viewer.background),
            text: a(p.diff_viewer.text),
            header_path: a(p.diff_viewer.header_path),
            muted: a(p.diff_viewer.muted),
            column_border: a(p.diff_viewer.column_border),
            gap_bg: a(p.diff_viewer.gap_bg),
            gap_fg: a(p.diff_viewer.gap_fg),
            removed_bg: a(p.diff_viewer.removed_bg),
            removed_fg: a(p.diff_viewer.removed_fg),
            added_bg: a(p.diff_viewer.added_bg),
            added_fg: a(p.diff_viewer.added_fg),
            changed_old_bg: a(p.diff_viewer.changed_old_bg),
            changed_old_fg: a(p.diff_viewer.changed_old_fg),
            changed_new_bg: a(p.diff_viewer.changed_new_bg),
            changed_new_fg: a(p.diff_viewer.changed_new_fg),
            line_number_fg: a(p.diff_viewer.line_number_fg),
            char_removed_bg: a(p.diff_viewer.char_removed_bg),
            char_removed_fg: a(p.diff_viewer.char_removed_fg),
            char_added_bg: a(p.diff_viewer.char_added_bg),
            char_added_fg: a(p.diff_viewer.char_added_fg),
        },
        toast: ToastPalette {
            background: a(p.toast.background),
            foreground: a(p.toast.foreground),
            alert_background: a(p.toast.alert_background),
            alert_foreground: a(p.toast.alert_foreground),
        },
    }
}

#[inline]
fn rgb_to_ansi256(
    r: u8,
    g: u8,
    b: u8,
) -> u8 {
    if r == g && g == b {
        if r < 8 {
            return 16;
        }
        if r > 248 {
            return 231;
        }
        return 232 + (r - 8) * 23 / 247;
    }
    let r6 = u16::from(r) * 5 / 255;
    let g6 = u16::from(g) * 5 / 255;
    let b6 = u16::from(b) * 5 / 255;
    (16 + 36 * r6 + 6 * g6 + b6) as u8
}

/// xterm-style RGB for index 0..=255 (only 16..231 and 232..255 used here).
fn ansi256_to_rgb(i: u8) -> (u8, u8, u8) {
    let i = i.min(255);
    if i >= 232 {
        let v = i - 232;
        let g = if v == 23 { 255 } else { 8 + v * 10 };
        return (g, g, g);
    }
    if i < 16 {
        const ANSI0_15: [(u8, u8, u8); 16] = [
            (0, 0, 0),
            (205, 0, 0),
            (0, 205, 0),
            (205, 205, 0),
            (0, 0, 238),
            (205, 0, 205),
            (0, 205, 205),
            (229, 229, 229),
            (127, 127, 127),
            (255, 0, 0),
            (0, 255, 0),
            (255, 255, 0),
            (92, 92, 255),
            (255, 0, 255),
            (0, 255, 255),
            (255, 255, 255),
        ];
        return ANSI0_15[i as usize];
    }
    let i = u16::from(i - 16);
    let r = (i / 36 % 6) * 51;
    let g = (i / 6 % 6) * 51;
    let b = (i % 6) * 51;
    (r as u8, g as u8, b as u8)
}

/// Euclidean nearest to the standard 16 ANSI foreground colors (xterm-ish).
fn rgb_to_ansi16_index(
    r: u8,
    g: u8,
    b: u8,
) -> u8 {
    const T: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let mut best = 0u8;
    let mut best_d = u32::MAX;
    for (idx, (tr, tg, tb)) in T.iter().enumerate() {
        let dr = i32::from(r) - i32::from(*tr);
        let dg = i32::from(g) - i32::from(*tg);
        let db = i32::from(b) - i32::from(*tb);
        let d = (dr * dr + dg * dg + db * db) as u32;
        if d < best_d {
            best_d = d;
            best = idx as u8;
        }
    }
    best
}

/// Best-effort sRGB for any [`Color`] (indexed and named colors use the xterm cube / table).
///
/// Used when deriving related colors (for example a line-number gutter stripe) so they stay
/// coherent after [`ColorDepth::adapt_color`] turns palette RGB into indexed ANSI colors.
pub fn theme_color_approx_rgb(c: Color) -> (u8, u8, u8) {
    ratatui_named_to_rgb(c)
}

fn ratatui_named_to_rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) => ansi256_to_rgb(i),
        Color::Reset => (192, 192, 192),
        Color::Black => (0, 0, 0),
        Color::Red => (205, 0, 0),
        Color::Green => (0, 205, 0),
        Color::Yellow => (205, 205, 0),
        Color::Blue => (0, 0, 238),
        Color::Magenta => (205, 0, 205),
        Color::Cyan => (0, 205, 205),
        Color::Gray => (192, 192, 192),
        Color::DarkGray => (127, 127, 127),
        Color::LightRed => (255, 0, 0),
        Color::LightGreen => (0, 255, 0),
        Color::LightYellow => (255, 255, 0),
        Color::LightBlue => (92, 92, 255),
        Color::LightMagenta => (255, 0, 255),
        Color::LightCyan => (0, 255, 255),
        Color::White => (255, 255, 255),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_to_256_black_white_corners() {
        assert_eq!(rgb_to_ansi256(0, 0, 0), 16);
        assert_eq!(rgb_to_ansi256(255, 255, 255), 231);
    }

    #[test]
    fn adapt_basic16_produces_indexed() {
        let d = ColorDepth::Basic16;
        assert!(matches!(
            d.adapt_color(Color::Rgb(10, 20, 80)),
            Color::Indexed(_)
        ));
    }

    /// Gutter stripe math (F4 editor) must not assume `Color::Rgb`: adapted palettes use indexed ANSI.
    #[test]
    fn derived_gutter_respects_color_depth() {
        let d = ColorDepth::Extended256;
        let main = d.adapt_color(Color::Rgb(30, 30, 35));
        assert!(
            matches!(main, Color::Indexed(_)),
            "sanity: non-truecolor main background is indexed"
        );
        let (r, g, b) = theme_color_approx_rgb(main);
        let gutter = d.adapt_color(Color::Rgb(
            r.saturating_add(10),
            g.saturating_add(10),
            b.saturating_add(12),
        ));
        assert!(
            matches!(gutter, Color::Indexed(_)),
            "gutter should remap through the same depth, not stay as mismatched truecolor"
        );
    }
}
