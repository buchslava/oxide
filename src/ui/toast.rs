//! Short-lived status toasts: timed message helpers.
//!
//! - **Ratatui:** [`TimedToast`] + [`draw_timed_bottom_left`].
//! - **Crossterm (main buffer):** use `app.ui_palette.toast` (see [`post_command_overlay`](crate::ui::post_command_overlay)).
//!   with [`post_command_overlay`](crate::ui::post_command_overlay).

use std::time::{Duration, Instant};

use ratatui::{layout::Rect, widgets::Paragraph, Frame};

use crate::ui::theme::UiPalette;

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
    palette: &UiPalette,
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
    let style = palette.toast.ratatui_style();
    f.render_widget(Paragraph::new(toast.message.as_str()).style(style), rect);
}
