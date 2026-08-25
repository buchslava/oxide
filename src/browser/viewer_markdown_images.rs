//! Embedded markdown images: [`ImageResolver`] + [`ratatui_image`] draw loop.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::DynamicImage;
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui029::prelude::Stylize;
use ratatui029::style::{Color, Style};
use ratatui029::text::Span;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{FilterType, Image, Resize};
use ratatui_markdown::markdown::{ImagePlacement, ImageResolver};

use super::viewer_image::picker_cell_font_size;

/// Delay after the last scroll before embedded images are drawn again.
pub const IMAGE_SCROLL_DEBOUNCE: Duration = Duration::from_millis(300);

/// Suppresses image draws while the user is scrolling; clears after [`IMAGE_SCROLL_DEBOUNCE`].
#[derive(Debug, Clone)]
pub struct MarkdownImageScrollGate {
    scroll_activity: Option<Instant>,
}

impl MarkdownImageScrollGate {
    pub fn new() -> Self {
        Self {
            scroll_activity: None,
        }
    }

    pub fn note_scroll(&mut self) {
        self.scroll_activity = Some(Instant::now());
    }

    /// Whether embedded images should be drawn this frame.
    pub fn should_draw(
        &mut self,
        has_images: bool,
    ) -> bool {
        if !has_images {
            return false;
        }
        match self.scroll_activity {
            None => true,
            Some(t) if t.elapsed() >= IMAGE_SCROLL_DEBOUNCE => {
                self.scroll_activity = None;
                true
            }
            Some(_) => false,
        }
    }
}

impl Default for MarkdownImageScrollGate {
    fn default() -> Self {
        Self::new()
    }
}

fn height_divisor(
    font_h: u16,
    proto: ProtocolType,
) -> f64 {
    match proto {
        ProtocolType::Halfblocks => font_h as f64 * 2.0,
        _ => font_h as f64,
    }
}

fn pixel_to_cell(
    pw: u32,
    ph: u32,
    font_w: u16,
    font_h: u16,
    proto: ProtocolType,
) -> (u16, u16) {
    if pw == 0 || ph == 0 || font_w == 0 {
        return (0, 0);
    }
    let cw = (pw as f64 / font_w as f64).ceil() as u16;
    let ch = (ph as f64 / height_divisor(font_h, proto)).ceil() as u16;
    (cw.max(1), ch.max(1))
}

fn rows_to_pixel_height(
    rows: u16,
    font_h: u16,
    proto: ProtocolType,
) -> u32 {
    (rows as f64 * height_divisor(font_h, proto)).ceil() as u32
}

/// Resolve `![alt](path)` relative to the markdown file's directory.
pub struct OxideMarkdownImageResolver {
    base_dir: PathBuf,
    font_w: u16,
    font_h: u16,
    protocol_type: ProtocolType,
}

impl OxideMarkdownImageResolver {
    pub fn new(
        base_dir: PathBuf,
        picker: &Picker,
    ) -> Self {
        let (font_w, font_h) = picker_cell_font_size(picker);
        Self {
            base_dir,
            font_w,
            font_h,
            protocol_type: picker.protocol_type(),
        }
    }
}

impl ImageResolver for OxideMarkdownImageResolver {
    fn resolve(
        &mut self,
        path: &str,
    ) -> Option<DynamicImage> {
        let full_path = self.base_dir.join(path);
        image::ImageReader::open(&full_path).ok()?.decode().ok()
    }

    fn cell_dimensions(
        &mut self,
        img: &DynamicImage,
        max_width: u16,
        max_height: u16,
    ) -> (u16, u16) {
        let (cw, ch) = pixel_to_cell(
            img.width(),
            img.height(),
            self.font_w,
            self.font_h,
            self.protocol_type,
        );
        let w = cw.min(max_width);
        let h = if w < cw {
            let ratio = img.height() as f64 * w as f64 / (img.width() as f64).max(1.0);
            (ratio / height_divisor(self.font_h, self.protocol_type)).ceil() as u16
        } else {
            ch
        };
        let h = h.min(max_height);
        (w.max(1), h.max(1))
    }

    fn fallback(
        &self,
        path: &str,
        alt: &str,
    ) -> Span<'static> {
        let label = if alt.is_empty() { path } else { alt };
        Span::styled(
            format!("[no image: {label}]"),
            Style::default().italic().fg(Color::Gray),
        )
    }
}

/// Signature of the visible crop — protocol is reused while this stays the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CropSignature {
    crop_l: u32,
    crop_t: u32,
    crop_r: u32,
    crop_b: u32,
    vis_w: u16,
    vis_h: u16,
}

/// One embedded image from [`render_full`], with cached [`Protocol`] for drawing.
pub struct EmbeddedMarkdownImage {
    pub doc_row: usize,
    pub col: usize,
    pub width_cells: u16,
    pub height_cells: u16,
    scaled: DynamicImage,
    /// Uncropped protocol — reused while the whole image fits in the viewport.
    full_protocol: Option<Protocol>,
    /// Cropped protocol for partial visibility at a specific scroll position.
    protocol: Option<Protocol>,
    cached_crop: Option<CropSignature>,
    pub failed: bool,
    /// Set when layout/resolution changes (not on scroll).
    dirty: bool,
}

impl EmbeddedMarkdownImage {
    pub fn from_placement(
        placement: ImagePlacement,
        picker: &Picker,
    ) -> Self {
        let (font_w, font_h) = picker_cell_font_size(picker);
        let proto = picker.protocol_type();
        let target_px_w = (placement.width_cells as u32 * font_w as u32).max(1);
        let target_px_h = rows_to_pixel_height(placement.height_cells, font_h, proto).max(1);
        let scaled = placement.image.resize_exact(
            target_px_w,
            target_px_h,
            FilterType::Triangle,
        );
        Self {
            doc_row: placement.row,
            col: placement.col,
            width_cells: placement.width_cells,
            height_cells: placement.height_cells,
            scaled,
            full_protocol: None,
            protocol: None,
            cached_crop: None,
            failed: false,
            dirty: true,
        }
    }
}

pub fn base_dir_for_markdown(file_path: &str) -> PathBuf {
    Path::new(file_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Draw visible embedded images over the markdown text viewport (scroll-aware clipping).
pub fn draw_markdown_images(
    f: &mut Frame,
    images: &mut [EmbeddedMarkdownImage],
    picker: &Picker,
    content_rect: Rect,
    scroll: u16,
) {
    let (font_w, font_h) = picker_cell_font_size(picker);
    let scroll_i = scroll as i32;
    let text_left = content_rect.x;
    let text_top = content_rect.y;
    let content_w = content_rect.width;
    let content_h = content_rect.height;

    let vp_l = text_left as i32;
    let vp_t = text_top as i32;
    let vp_r = (text_left as i32 + content_w as i32 - 1).max(vp_l);
    let vp_b = (text_top as i32 + content_h as i32 - 1).max(vp_t);

    for img in images {
        if img.failed || img.width_cells == 0 || img.height_cells == 0 {
            continue;
        }

        let img_l = text_left as i32 + img.col as i32;
        let img_t = text_top as i32 + img.doc_row as i32 - scroll_i;
        let img_r = img_l + img.width_cells as i32 - 1;
        let img_b = img_t + img.height_cells as i32 - 1;

        if img_r < vp_l || img_l > vp_r || img_b < vp_t || img_t > vp_b {
            continue;
        }

        let clip_l = img_l.max(vp_l);
        let clip_t = img_t.max(vp_t);
        let clip_r = img_r.min(vp_r);
        let clip_b = img_b.min(vp_b);

        let vis_w = (clip_r - clip_l + 1) as u16;
        let vis_h = (clip_b - clip_t + 1) as u16;

        let crop_cells_l = (clip_l - img_l) as u32;
        let crop_cells_t = (clip_t - img_t) as u32;
        let crop_cells_r = (img_r - clip_r) as u32;
        let crop_cells_b = (img_b - clip_b) as u32;

        let fw = font_w as u32;
        let fh = font_h as u32;
        let total_px_w = img.scaled.width();
        let total_px_h = img.scaled.height();

        let crop_sig = CropSignature {
            crop_l: crop_cells_l,
            crop_t: crop_cells_t,
            crop_r: crop_cells_r,
            crop_b: crop_cells_b,
            vis_w,
            vis_h,
        };

        let fully_visible = img_l >= vp_l && img_t >= vp_t && img_r <= vp_r && img_b <= vp_b;

        let proto_ref = if fully_visible {
            let need_build = img.dirty || img.full_protocol.is_none();
            if need_build {
                let rect_for_proto = Rect::new(0, 0, img.width_cells, img.height_cells);
                match picker.new_protocol(
                    img.scaled.clone(),
                    rect_for_proto,
                    Resize::Fit(None),
                ) {
                    Ok(proto) => {
                        img.full_protocol = Some(proto);
                        img.protocol = None;
                        img.cached_crop = None;
                        img.dirty = false;
                    }
                    Err(_) => {
                        img.failed = true;
                        continue;
                    }
                }
            }
            match &img.full_protocol {
                Some(p) => p,
                None => continue,
            }
        } else {
            let need_build = img.dirty || img.cached_crop != Some(crop_sig);
            if need_build {
                let crop_px_x = crop_cells_l * fw;
                let crop_px_y = crop_cells_t * fh;
                let crop_px_w = total_px_w
                    .saturating_sub(crop_cells_l * fw)
                    .saturating_sub(crop_cells_r * fw)
                    .max(1);
                let crop_px_h = total_px_h
                    .saturating_sub(crop_cells_t * fh)
                    .saturating_sub(crop_cells_b * fh)
                    .max(1);

                let need_crop =
                    crop_cells_l > 0 || crop_cells_t > 0 || crop_cells_r > 0 || crop_cells_b > 0;

                let img_for_proto = if need_crop {
                    img.scaled
                        .crop_imm(crop_px_x, crop_px_y, crop_px_w, crop_px_h)
                } else {
                    img.scaled.clone()
                };

                let rect_for_proto = Rect::new(0, 0, vis_w, vis_h);
                match picker.new_protocol(
                    img_for_proto,
                    rect_for_proto,
                    Resize::Fit(None),
                ) {
                    Ok(proto) => {
                        img.protocol = Some(proto);
                        img.cached_crop = Some(crop_sig);
                        img.dirty = false;
                    }
                    Err(_) => {
                        img.failed = true;
                        continue;
                    }
                }
            }
            match &img.protocol {
                Some(p) => p,
                None => continue,
            }
        };

        let rect = if fully_visible {
            Rect::new(
                img_l as u16,
                img_t as u16,
                img.width_cells,
                img.height_cells,
            )
        } else {
            Rect::new(clip_l as u16, clip_t as u16, vis_w, vis_h)
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let widget = Image::new(proto_ref);
            f.render_widget(widget, rect);
        }));
        if result.is_err() {
            img.failed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_gate_draws_when_idle() {
        let mut gate = MarkdownImageScrollGate::new();
        assert!(gate.should_draw(true));
    }

    #[test]
    fn scroll_gate_hides_while_scrolling() {
        let mut gate = MarkdownImageScrollGate::new();
        gate.note_scroll();
        assert!(!gate.should_draw(true));
    }

    #[test]
    fn scroll_gate_skips_when_no_images() {
        let mut gate = MarkdownImageScrollGate::new();
        assert!(!gate.should_draw(false));
    }

    #[test]
    fn resolver_joins_relative_to_base_dir() {
        let picker = Picker::halfblocks();
        let base = PathBuf::from("/tmp/docs");
        let mut resolver = OxideMarkdownImageResolver::new(base.clone(), &picker);
        // Nonexistent path — resolve returns None without panicking.
        assert!(resolver.resolve("images/missing.png").is_none());
        assert_eq!(resolver.base_dir, base);
    }

    #[test]
    fn cell_dimensions_respects_max_width() {
        let picker = Picker::halfblocks();
        let mut resolver = OxideMarkdownImageResolver::new(PathBuf::from("."), &picker);
        let img = DynamicImage::new_rgb8(900, 600);
        let (w, h) = resolver.cell_dimensions(&img, 20, 999);
        assert!(w <= 20);
        assert!(w >= 1);
        assert!(h >= 1);
    }
}
