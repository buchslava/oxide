//! Oxide shortcuts use a **Ctrl+X** prefix, then the former second key (e.g. Ctrl+X then `F` for find).
//! **Ctrl+O** (suspend) is centralized in [`is_direct_ctrl_o_suspend`] / [`poll_suspend_chord`]; dialogs use those.
//! **Ctrl+R** refresh and command-line **Ctrl+C**/**Ctrl+V** are handled in [`crate::app::events::EventHandler`]
//! so this module stays “chord + suspend-from-dialog” only. Text fields keep **Ctrl+A** for select-all.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app::state::AppState;

/// Classic **Ctrl+O** (SI, `0x0F`) or `Ctrl`+`O` — return to panels / open subshell without the Ctrl+X prefix.
#[inline]
pub fn is_direct_ctrl_o_suspend(
    code: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    modifiers.contains(KeyModifiers::CONTROL)
        && matches!(code, KeyCode::Char('\x0f' | 'o' | 'O'))
}

#[inline]
pub fn is_ctrl_x_prefix(
    code: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    modifiers.contains(KeyModifiers::CONTROL)
        && matches!(
            code,
            KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Char('\x18')
        )
}

/// **Ctrl+X** then **O**: suspend to subshell (in addition to single-stroke **Ctrl+O**, see [`is_direct_ctrl_o_suspend`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendChordResult {
    /// Not a chord key; caller handles `code` normally.
    NotHandled,
    /// Chord started, cancelled, or unknown second key after clearing pending (caller may still handle `code`).
    Consumed,
    SuspendToShell,
}

pub fn poll_suspend_chord(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> SuspendChordResult {
    if is_direct_ctrl_o_suspend(code, modifiers) {
        app.ctrl_x_chord_pending = false;
        return SuspendChordResult::SuspendToShell;
    }
    if app.ctrl_x_chord_pending {
        app.ctrl_x_chord_pending = false;
        if code == KeyCode::Esc {
            return SuspendChordResult::Consumed;
        }
        if is_ctrl_x_prefix(code, modifiers) {
            return SuspendChordResult::Consumed;
        }
        if matches!(code, KeyCode::Char('o' | 'O')) {
            return SuspendChordResult::SuspendToShell;
        }
        return SuspendChordResult::NotHandled;
    }
    if is_ctrl_x_prefix(code, modifiers) {
        app.ctrl_x_chord_pending = true;
        return SuspendChordResult::Consumed;
    }
    SuspendChordResult::NotHandled
}
