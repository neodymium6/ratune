//! Bounded raster prep for `ratatui-image` and Kitty album art.
//!
//! Covers are **contain**-fit into the widget cell rect (aspect-aware using font pixel size).
//! The draw rect is shrunk via [`contain_fit_rect_in_cells`] so gutters are not painted; bitmaps
//! are scaled with [`prepare_art_image_for_rect_contain_fit`] without a letterbox canvas.

use image::{imageops::FilterType, DynamicImage};
use ratatui::layout::Rect;
use ratatui_image::FontSize;

/// Cap per edge when scaling bitmaps (avoid huge protocol payloads).
pub const MAX_ART_EDGE_PX: u32 = 1024;

/// Pixel size of `inner` in terminal pixels, capped per edge.
pub fn pixel_budget_for_rect(inner: Rect, font: FontSize) -> (u32, u32) {
    let w = (inner.width as u32 * font.0 as u32).min(MAX_ART_EDGE_PX);
    let h = (inner.height as u32 * font.1 as u32).min(MAX_ART_EDGE_PX);
    (w.max(1), h.max(1))
}

/// Scale (up or down) so the image fits inside `max_w × max_h` while preserving aspect ratio.
pub fn fit_image_to_pixel_budget(img: DynamicImage, max_w: u32, max_h: u32) -> DynamicImage {
    let (iw, ih) = (img.width(), img.height());
    if iw == 0 || ih == 0 {
        return img;
    }
    let (tw, th) = fit_inside_scaled(iw, ih, max_w, max_h, false);
    if (tw, th) == (iw, ih) {
        return img;
    }
    img.resize_exact(tw, th, FilterType::Triangle)
}

fn fit_inside_scaled(w: u32, h: u32, max_w: u32, max_h: u32, allow_upscale: bool) -> (u32, u32) {
    let wratio = max_w as f64 / w as f64;
    let hratio = max_h as f64 / h as f64;
    let mut ratio = f64::min(wratio, hratio);
    if !allow_upscale {
        ratio = ratio.min(1.0);
    }
    let nw = ((w as f64 * ratio).round() as u32).max(1);
    let nh = ((h as f64 * ratio).round() as u32).max(1);
    (nw, nh)
}

/// Terminal-cell [`Rect`] inside `inner` that **contain**-fits `img`, centered (integer cols/rows).
///
/// Uses `font` so aspect ratio matches terminal **pixels** (cells are rarely square in px).
/// Used so album art is drawn only in the cells the cover occupies — gutters stay unpainted.
pub fn contain_fit_rect_in_cells(img: &DynamicImage, inner: Rect, font: FontSize) -> Rect {
    contain_fit_rect(img, inner, font, false)
}

/// Now Playing uses half the panel width and height, regardless of cover resolution.
/// Preserve aspect ratio and center the image; never crop or stretch it.
/// Home thumbnails retain their existing no-upscale placement policy.
pub fn now_playing_art_rect(img: &DynamicImage, inner: Rect, font: FontSize) -> Rect {
    if inner.width == 0 || inner.height == 0 {
        return inner;
    }
    let width = (inner.width / 2).max(1);
    let height = (inner.height / 2).max(1);
    let bounds = Rect::new(
        inner.x + (inner.width - width) / 2,
        inner.y + (inner.height - height) / 2,
        width,
        height,
    );
    let fit = contain_fit_rect(img, bounds, font, true);
    // Center against the original panel to avoid two rounds of cell rounding.
    Rect::new(
        inner.x + (inner.width - fit.width) / 2,
        inner.y + (inner.height - fit.height) / 2,
        fit.width,
        fit.height,
    )
}

fn contain_fit_rect(img: &DynamicImage, inner: Rect, font: FontSize, allow_upscale: bool) -> Rect {
    let (iw, ih) = (img.width(), img.height());
    if iw == 0 || ih == 0 || inner.width == 0 || inner.height == 0 {
        return inner;
    }
    let fw = font.0 as u32;
    let fh = font.1 as u32;
    if fw == 0 || fh == 0 {
        return inner;
    }
    let max_w_px = inner.width as u32 * fw;
    let max_h_px = inner.height as u32 * fh;
    let (fit_w_px, fit_h_px) = fit_inside_scaled(iw, ih, max_w_px, max_h_px, allow_upscale);

    let w = fit_w_px.div_ceil(fw).max(1).min(inner.width as u32) as u16;
    let h = fit_h_px.div_ceil(fh).max(1).min(inner.height as u32) as u16;

    let x = inner.x + (inner.width.saturating_sub(w)) / 2;
    let y = inner.y + (inner.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Contain-fit into the pixel budget for `rect` — no letterbox canvas (bitmap matches fitted size).
pub fn prepare_art_image_for_rect_contain_fit(
    img: DynamicImage,
    rect: Rect,
    font: FontSize,
) -> DynamicImage {
    let (max_w, max_h) = pixel_budget_for_rect(rect, font);
    fit_image_to_pixel_budget(img, max_w, max_h)
}

/// FNV-1a 64-bit digest of raw image bytes.
///
/// Used for Now Playing cache keys so consecutive tracks with different `cover_id` but identical
/// pixels do not trigger re-encode / re-transmit.
pub fn art_bytes_fingerprint(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 14695981039346656037;
    const PRIME: u64 = 1099511628211;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Decode cover bytes (JPEG, PNG, ICO, …) for display.
#[must_use]
pub fn art_bytes_decode(bytes: &[u8]) -> Option<DynamicImage> {
    image::load_from_memory(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn solid(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::new(w, h))
    }

    #[test]
    fn contain_fit_rect_square_in_wide_panel() {
        let inner = Rect::new(0, 0, 20, 10);
        let img = solid(500, 500);
        let font = (10u16, 10u16);
        let fit = contain_fit_rect_in_cells(&img, inner, font);
        assert_eq!(fit.width, 10);
        assert_eq!(fit.height, 10);
        assert_eq!(fit.x, 5);
        assert_eq!(fit.y, 0);
    }

    #[test]
    fn contain_fit_rect_wide_image_in_tall_panel() {
        let inner = Rect::new(0, 0, 10, 20);
        let img = solid(800, 400);
        let font = (10u16, 10u16);
        let fit = contain_fit_rect_in_cells(&img, inner, font);
        assert_eq!(fit.width, 10);
        assert_eq!(fit.height, 5);
        assert_eq!(fit.x, 0);
        assert_eq!(fit.y, 7);
    }

    #[test]
    fn contain_fit_rect_square_fills_tall_cells_when_px_aspect_matches() {
        // 20×10 cells at 10×20 px/cell → 200×200 px panel; square cover fills it.
        let inner = Rect::new(0, 0, 20, 10);
        let img = solid(500, 500);
        let font = (10u16, 20u16);
        let fit = contain_fit_rect_in_cells(&img, inner, font);
        assert_eq!(fit, inner);
    }

    #[test]
    fn contain_fit_rect_small_icon_not_upscaled() {
        let inner = Rect::new(0, 0, 20, 10);
        let img = solid(48, 48);
        let font = (10u16, 10u16);
        let fit = contain_fit_rect_in_cells(&img, inner, font);
        assert_eq!(fit.width, 5);
        assert_eq!(fit.height, 5);
        assert_eq!(fit.x, 7);
        assert_eq!(fit.y, 2);
    }

    #[test]
    fn fit_image_does_not_upscale_small_sources() {
        let img = solid(48, 48);
        let out = fit_image_to_pixel_budget(img, 400, 400);
        assert_eq!(out.width(), 48);
        assert_eq!(out.height(), 48);
    }

    #[test]
    fn now_playing_square_size_is_independent_of_source_resolution() {
        let inner = Rect::new(5, 3, 80, 40);
        let font = (10, 20); // 800 × 800 pixels.
        for size in [48, 128, 300, 600, 1200] {
            let fit = now_playing_art_rect(&solid(size, size), inner, font);
            assert_eq!(fit, Rect::new(25, 13, 40, 20), "source {size} × {size}");
        }
    }

    #[test]
    fn now_playing_rectangular_covers_keep_aspect_ratio_and_centering() {
        let inner = Rect::new(5, 3, 80, 40);
        let font = (10, 20);
        for size in [48, 128, 300, 600] {
            let wide = now_playing_art_rect(&solid(size * 2, size), inner, font);
            assert_eq!(wide, Rect::new(25, 18, 40, 10));
            let tall = now_playing_art_rect(&solid(size, size * 2), inner, font);
            assert_eq!(tall, Rect::new(35, 13, 20, 20));
        }
    }

    #[test]
    fn now_playing_handles_non_square_cells_and_empty_bounds() {
        let img = solid(128, 128);
        let inner = Rect::new(1, 1, 105, 58);
        let fit = now_playing_art_rect(&img, inner, (13, 27));
        assert_eq!(fit, Rect::new(27, 17, 52, 26));
        for empty in [Rect::new(1, 1, 0, 58), Rect::new(1, 1, 105, 0)] {
            assert_eq!(now_playing_art_rect(&img, empty, (13, 27)), empty);
        }
        let half = Rect::new(27, 15, 52, 29);
        assert_eq!(now_playing_art_rect(&img, inner, (0, 27)), half);
        assert_eq!(now_playing_art_rect(&solid(0, 0), inner, (13, 27)), half);
    }

    #[test]
    fn now_playing_iterm2_payload_dimensions_do_not_depend_on_source_resolution() {
        use image::Rgba;
        use ratatui::buffer::Buffer;
        use ratatui_image::{
            protocol::{iterm2::Iterm2, ImageSource, StatefulProtocol, StatefulProtocolType},
            Resize, ResizeEncodeRender,
        };

        let inner = Rect::new(1, 1, 60, 30);
        let font = (10, 20);
        for size in [128, 600, 1200] {
            let img = solid(size, size);
            let rect = now_playing_art_rect(&img, inner, font);
            // Follow the same prepare -> resize/encode path as Now Playing's worker.
            let prepared = prepare_art_image_for_rect_contain_fit(img, rect, font);
            let mut protocol = StatefulProtocol::new(
                ImageSource::new(prepared, font, Rgba([0, 0, 0, 255])),
                font,
                StatefulProtocolType::ITerm2(Iterm2 {
                    is_tmux: true,
                    ..Iterm2::default()
                }),
            );
            protocol.resize_encode_render(
                &Resize::Scale(Some(FilterType::Triangle)),
                rect,
                &mut Buffer::empty(inner),
            );
            protocol.last_encoding_result().unwrap().unwrap();
            let StatefulProtocolType::ITerm2(encoded) = protocol.protocol_type() else {
                panic!("expected iTerm2");
            };
            assert_eq!(encoded.area, Rect::new(0, 0, 30, 15));
            assert!(encoded.data.contains(";width=300px;height=300px;"));
        }
    }

    #[test]
    fn decode_ico_favicon_file() {
        let bytes = std::fs::read("/tmp/favicon.ico").unwrap_or_default();
        if bytes.len() < 6 {
            return;
        }
        let img = art_bytes_decode(&bytes).expect("ico decode");
        assert!(img.width() > 0 && img.height() > 0);
    }
}
