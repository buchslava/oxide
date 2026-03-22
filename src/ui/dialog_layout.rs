//! Shared layout helpers for centered modal dialogs. Single source of truth for rect/content math.

use ratatui::layout::{Margin, Rect};

/// Default horizontal padding inside dialog content (used by most dialogs).
pub const DEFAULT_PAD_H: u16 = 2;

/// Margin from terminal edge when centering (so dialog doesn't touch sides).
const AREA_MARGIN: u16 = 4;

/// Return a centered dialog rect within `area` with given max width and height.
#[must_use]
pub fn centered_dialog_rect(
    area: Rect,
    max_width: u16,
    height: u16,
) -> Rect {
    let w = max_width.min(area.width.saturating_sub(AREA_MARGIN));
    let h = height.min(area.height.saturating_sub(2));
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Return the content rect inside a dialog (inside borders, with horizontal padding).
#[must_use]
pub fn dialog_content_rect(
    rect: Rect,
    pad_h: u16,
) -> Rect {
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    Rect {
        x: inner.x + pad_h,
        y: inner.y,
        width: inner.width.saturating_sub(pad_h * 2),
        height: inner.height,
    }
}

/// Standard size for single-input dialogs (mkdir, archive, new file).
pub const SINGLE_INPUT_DIALOG_WIDTH: u16 = 52;
pub const SINGLE_INPUT_DIALOG_HEIGHT: u16 = 9;

/// Full layout for a single-input dialog: (dialog_rect, content_rect).
#[must_use]
pub fn single_input_dialog_layout(area: Rect) -> (Rect, Rect) {
    let rect = centered_dialog_rect(area, SINGLE_INPUT_DIALOG_WIDTH, SINGLE_INPUT_DIALOG_HEIGHT);
    let content = dialog_content_rect(rect, DEFAULT_PAD_H);
    (rect, content)
}

/// Return (create_button_rect, cancel_button_rect) for hit-testing. Same layout as single-input dialogs.
#[must_use]
pub fn single_input_dialog_button_rects(area: Rect) -> (Rect, Rect) {
    let (_, content) = single_input_dialog_layout(area);
    single_input_button_rects(content)
}

/// Return (create_button_rect, cancel_button_rect) for a single-input dialog (Create/Cancel).
/// Uses standard widths: Create 10, Cancel 10, gap 4. Buttons centered in content, at content.y + 5.
#[must_use]
pub fn single_input_button_rects(content: Rect) -> (Rect, Rect) {
    const CREATE_W: u16 = 10;
    const CANCEL_W: u16 = 10;
    const BTN_GAP: u16 = 4;
    let total = CREATE_W + CANCEL_W + BTN_GAP;
    let btn_start_x = content.x + content.width.saturating_sub(total) / 2;
    let btn_y = content.y + 5;
    (
        Rect {
            x: btn_start_x,
            y: btn_y,
            width: CREATE_W,
            height: 1,
        },
        Rect {
            x: btn_start_x + CREATE_W + BTN_GAP,
            y: btn_y,
            width: CANCEL_W,
            height: 1,
        },
    )
}

/// Return two button rects (left and right) centered in content. Used for Yes/No, OK, etc.
#[must_use]
pub fn two_button_rects(
    content: Rect,
    button_y: u16,
    left_width: u16,
    right_width: u16,
    gap: u16,
) -> (Rect, Rect) {
    let total = left_width + right_width + gap;
    let start_x = content.x + content.width.saturating_sub(total) / 2;
    (
        Rect {
            x: start_x,
            y: button_y,
            width: left_width,
            height: 1,
        },
        Rect {
            x: start_x + left_width + gap,
            y: button_y,
            width: right_width,
            height: 1,
        },
    )
}
