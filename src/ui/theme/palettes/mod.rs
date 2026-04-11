//! Semantic color **types** and small behavior (`fill_style`, …).
//! Concrete RGB tables live in [`crate::ui::theme::themes`] (one module per preset).

mod chrome;
mod color;
mod dialog;
mod diff_viewer;
mod panel_list;
mod progress;
mod toast;
mod viewer;

pub use chrome::ChromePalette;
pub use color::rgb;
pub use dialog::DialogPalette;
pub use diff_viewer::DiffViewerPalette;
pub use panel_list::PanelListPalette;
pub use progress::ProgressPalette;
pub use toast::ToastPalette;
pub use viewer::ViewerPalette;
