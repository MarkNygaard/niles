//! The card's picture: Niles's own copy of the screenshot, re-encoded,
//! with the outline drawn by Niles and never on the page, so the page
//! can't move or remove it.

use image::ImageFormat;

use crate::error::{Error, Result};
use crate::tab::Rect;

/// The outline's colour and thickness.
const OUTLINE: [u8; 3] = [225, 29, 72];
const THICKNESS: u32 = 3;

/// How far outside the element the outline sits.
const MARGIN: f64 = 4.0;

/// No picture is bigger than this; a page that large is refused rather
/// than kept in memory.
const MAX_PIXELS: u32 = 4096;

/// Draws `outline` on the PNG `screenshot`, when there is one, and
/// returns a JPEG: Niles's own copy, re-encoded, never what came off
/// the page.
pub fn picture(screenshot: &[u8], outline: Option<Rect>) -> Result<Vec<u8>> {
    let decoded =
        image::load_from_memory_with_format(screenshot, ImageFormat::Png).map_err(|e| {
            Error::Picture {
                reason: format!("couldn't read the screenshot: {e}"),
            }
        })?;
    if decoded.width() > MAX_PIXELS || decoded.height() > MAX_PIXELS {
        return Err(Error::Picture {
            reason: format!(
                "{}×{} is over {MAX_PIXELS}×{MAX_PIXELS}",
                decoded.width(),
                decoded.height()
            ),
        });
    }
    let mut rgb = decoded.to_rgb8();
    if let Some(outline) = outline {
        draw(&mut rgb, outline);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&rgb)
        .map_err(|e| Error::Picture {
            reason: format!("couldn't encode the JPEG: {e}"),
        })?;
    Ok(out.into_inner())
}

/// Draws the outline `THICKNESS` pixels thick, `MARGIN` outside
/// `outline`, clamped to the image; nothing when it lies wholly outside.
fn draw(image: &mut image::RgbImage, outline: Rect) {
    let (w, h) = (image.width() as f64, image.height() as f64);
    let x0 = (outline.x - MARGIN).max(0.0) as u32;
    let y0 = (outline.y - MARGIN).max(0.0) as u32;
    let x1 = (outline.x + outline.width + MARGIN).ceil().min(w) as u32;
    let y1 = (outline.y + outline.height + MARGIN).ceil().min(h) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for layer in 0..THICKNESS.min(x1 - x0).min(y1 - y0) {
        for x in x0..x1 - layer {
            image.put_pixel(x, y0 + layer, image::Rgb(OUTLINE));
            image.put_pixel(x, y1 - 1 - layer, image::Rgb(OUTLINE));
        }
        for y in y0..y1 - layer {
            image.put_pixel(x0 + layer, y, image::Rgb(OUTLINE));
            image.put_pixel(x1 - 1 - layer, y, image::Rgb(OUTLINE));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white PNG of `width`×`height`.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([250, 250, 250]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut out, ImageFormat::Png)
            .expect("png encode");
        out.into_inner()
    }

    #[test]
    fn a_jpeg_comes_out_with_the_outline_drawn() {
        let shot = png(200, 100);
        let jpeg = picture(
            &shot,
            Some(Rect {
                x: 50.0,
                y: 20.0,
                width: 40.0,
                height: 20.0,
            }),
        )
        .expect("picture");
        assert_eq!(jpeg[..2], [0xFF, 0xD8]);
        let back = image::load_from_memory(&jpeg).expect("jpeg decodes");
        let rgb = back.to_rgb8();
        // A pixel on the outline, 4 px outside the element's left edge.
        let on = rgb.get_pixel(47, 30);
        assert!(
            on[0] > 150 && on[1] < 100 && on[2] < 120,
            "outline is red: {on:?}"
        );
        // A pixel far away stays near white (allowing for JPEG).
        let off = rgb.get_pixel(190, 90);
        assert!(
            off[0] > 200 && off[1] > 200 && off[2] > 200,
            "far pixel is white: {off:?}"
        );
    }

    #[test]
    fn a_picture_without_an_outline_has_no_red() {
        let shot = png(64, 48);
        let out = picture(&shot, None).expect("picture");
        let back = image::load_from_memory(&out).expect("jpeg decodes");
        assert!(
            back.to_rgb8().pixels().all(|p| {
                // JPEG smears a little: red needs a clear margin.
                p[0] < 150 || p[1] > 80 || p[2] > 100
            }),
            "no outline was drawn"
        );
    }

    #[test]
    fn an_outline_off_the_image_draws_nothing() {
        let shot = png(64, 48);
        let out = picture(
            &shot,
            Some(Rect {
                x: 1000.0,
                y: 1000.0,
                width: 10.0,
                height: 10.0,
            }),
        )
        .expect("picture");
        let back = image::load_from_memory(&out).expect("jpeg decodes");
        assert!(
            back.to_rgb8().pixels().all(|p| p[0] > 200),
            "nothing was drawn"
        );
    }

    #[test]
    fn an_oversized_image_is_refused() {
        let shot = png(MAX_PIXELS + 1, 10);
        match picture(
            &shot,
            Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 5.0,
                height: 5.0,
            }),
        ) {
            Err(Error::Picture { .. }) => {}
            other => panic!("expected Picture, got {other:?}"),
        }
    }
}
