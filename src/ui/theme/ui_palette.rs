//! Root bundle passed through the app each frame.

use super::palettes::{
    ChromePalette, DialogPalette, DiffViewerPalette, PanelListPalette, ProgressPalette,
    ToastPalette, ViewerPalette,
};

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
