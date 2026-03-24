//! Application-wide color system for ratatui and crossterm.
//!
//! - **[`UiPalette`]** — all colors as plain data (`Copy`). Stored on [`crate::app::state::AppState`]
//!   and passed (or read from `app`) wherever widgets are drawn.
//! - **[`ThemeId`]** — closed set of built-in themes; extend this when adding presets. A future
//!   “theme manager” can map persisted names → [`ThemeId`] or custom [`UiPalette`] values.

mod palette;

pub use palette::{DialogPalette, PanelListPalette, ThemeId, UiPalette, ViewerPalette};
