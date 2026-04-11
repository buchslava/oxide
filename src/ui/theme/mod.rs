//! Application-wide color system for ratatui and crossterm.
//!
//! - **[`UiPalette`]** — all colors as plain data (`Copy`). Stored on [`crate::app::state::AppState`]
//!   and passed wherever widgets are drawn.
//! - **[`ThemeId`]** — closed set of built-in themes. Tables live under [`themes`]; semantic types
//!   and helpers under [`palettes`]. Persisted slug → [`ThemeId::from_slug`].

pub(crate) mod palettes;
mod theme_id;
mod themes;
mod ui_palette;

pub use palettes::{DialogPalette, DiffViewerPalette, PanelListPalette, ViewerPalette};
pub use theme_id::ThemeId;
pub use ui_palette::UiPalette;
