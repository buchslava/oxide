//! Short-lived status toasts: shared palette and timed message helpers.
//!
//! - **Ratatui:** [`TimedToast`] + [`draw_timed_bottom_left`] for one-line overlays (e.g. editor).
//! - **Crossterm (main buffer):** use [`palette_crossterm_bg`] / [`palette_crossterm_fg`] with
//!   [`post_command_overlay`](crate::post_command_overlay) so colors stay consistent.

use std::time::{Duration, Instant};

use crossterm::style::Color as CrosstermColor;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::Paragraph,
    Frame,
};

// --- Palette (single source for ratatui + crossterm) ---

const BG_R: u8 = 60;
const BG_G: u8 = 60;
const BG_B: u8 = 60;
const FG_R: u8 = 140;
const FG_G: u8 = 200;
const FG_B: u8 = 140;

#[inline]
pub fn palette_ratatui_bg() -> Color {
    Color::Rgb(BG_R, BG_G, BG_B)
}

#[inline]
pub fn palette_ratatui_fg() -> Color {
    Color::Rgb(FG_R, FG_G, FG_B)
}

#[inline]
pub fn palette_crossterm_bg() -> CrosstermColor {
    CrosstermColor::Rgb {
        r: BG_R,
        g: BG_G,
        b: BG_B,
    }
}

#[inline]
pub fn palette_crossterm_fg() -> CrosstermColor {
    CrosstermColor::Rgb {
        r: FG_R,
        g: FG_G,
        b: FG_B,
    }
}

// --- Timed overlay (ratatui) ---

/// One-line message shown until [`Self::until`].
#[derive(Debug, Clone)]
pub struct TimedToast {
    pub until: Instant,
    pub message: String,
}

impl TimedToast {
    pub fn new(
        duration: Duration,
        message: String,
    ) -> Self {
        Self {
            until: Instant::now() + duration,
            message,
        }
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.until
    }

    /// Drop the toast from `slot` when past its deadline.
    pub fn clear_if_expired(slot: &mut Option<Self>) {
        if slot.as_ref().is_some_and(Self::is_expired) {
            *slot = None;
        }
    }
}

/// Bottom row, left (`area` = full host rect). Draw after other bottom-row widgets so it stacks on top.
pub fn draw_timed_bottom_left(
    f: &mut Frame,
    area: Rect,
    toast: &TimedToast,
) {
    const PAD: u16 = 1;
    let w = (toast.message.chars().count() as u16)
        .min(area.width.saturating_sub(PAD))
        .max(1);
    let row = area.bottom().saturating_sub(1);
    let rect = Rect {
        x: area.x + PAD,
        y: row,
        width: w,
        height: 1,
    };
    let style = Style::default()
        .bg(palette_ratatui_bg())
        .fg(palette_ratatui_fg());
    f.render_widget(Paragraph::new(toast.message.as_str()).style(style), rect);
}
