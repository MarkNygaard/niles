//! Pictures an artifact keeps: Niles's own copies of pictures found on
//! the web, for a trip's days, a document, a page.
//!
//! A copy, never an address. A page has no network, so a picture it
//! draws has to be handed to it; and an address in a view would be a
//! request from the person's phone to whoever runs that site, each time
//! they look, which can say who is looking and when, or carry what the
//! page knows away in the address. Sites take pictures down besides. So
//! Niles fetches the picture once (`niles-websearch`'s fetcher, which
//! goes only to the public internet), and keeps it here.
//!
//! What it keeps is not what it fetched. The bytes are sniffed (JPEG,
//! PNG, WebP or GIF; never SVG, which is a document that can run
//! script), decoded under limits that stop a small file that opens into
//! gigabytes, turned the way the camera said, made at most
//! [`MAX_SIDE`] pixels on the long side, and written again: a JPEG, or
//! a PNG when it has transparency. Writing it again drops everything
//! that was not the picture, EXIF with its GPS position among it, and
//! leaves nothing of a file that was also something else.
//!
//! Content names a picture as `niles-image:12` ([`SCHEME`]), by the id
//! [`crate::ArtifactStore::add_image`] gave it. A name of one that is not
//! there (taken away, or of a version from before it) is not refused: it
//! shows as nothing, so restoring an old version or ticking a box never
//! fails over a picture.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::io::Cursor;

/// How content names a picture this artifact keeps: `niles-image:12`.
pub const SCHEME: &str = "niles-image:";

/// Pictures one artifact keeps: a trip of three weeks with one for each
/// day and a few more.
pub const MAX_PER_ARTIFACT: usize = 40;
/// What they come to together, as kept. A picture as kept is a few
/// hundred KB, so the count is what is usually reached first.
pub const MAX_BYTES_PER_ARTIFACT: usize = 20_000_000;
/// The most Niles downloads for one picture before it stops: a camera's
/// full-size JPEG fits, a video does not.
pub const MAX_DOWNLOAD: usize = 15 * 1024 * 1024;
/// The long side of a picture as kept: sharp across a phone at 3×, and
/// across a laptop's panel.
pub const MAX_SIDE: u32 = 1600;
/// What a decoded picture may be, whatever its file says: a 32
/// megapixel photo opens; a 1 KB file claiming 60,000 pixels square,
/// which would take gigabytes to open, does not.
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 32_000_000;
const MAX_ALLOC: u64 = 160 * 1024 * 1024;
const JPEG_QUALITY: u8 = 80;

pub const MAX_ALT: usize = 300;
pub const MAX_CREDIT: usize = 200;
const MAX_SOURCE: usize = 2_000;

/// What the model is told about pictures: how one is found, kept and
/// named, for each kind's help and the jobs' instructions.
pub const HELP: &str = "Pictures: find one with image_search where it is offered, then keep a \
    copy with artifact_add_image {id, url, alt, credit, source}, which answers its name, \
    niles-image:ID. Use that name, never a web address: a picture from an address is never \
    shown. Always give alt (what the picture shows, for someone who cannot see it) and \
    credit (who took or owns it, as the result says, or the site it is from). A few good \
    pictures are better than many: one for the whole, one for a day or a place that \
    deserves it.";

/// The four formats a picture is taken in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    Gif,
}

impl Format {
    fn image(self) -> ImageFormat {
        match self {
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
            Self::Webp => ImageFormat::WebP,
            Self::Gif => ImageFormat::Gif,
        }
    }
}

/// What the first bytes say a file is, of the formats taken; `None` for
/// anything else, whatever its address or its server said.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(Format::Webp)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else {
        None
    }
}

fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid {
        kind: "image",
        reason: reason.into(),
    }
}

/// Why a file that is not one of the four was not taken, said for the
/// model choosing another.
fn not_a_picture(bytes: &[u8]) -> Error {
    let start = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase();
    let start = start.trim_start_matches('\u{feff}').trim_start();
    if start.contains("<svg") {
        return invalid(
            "it is an SVG drawing, which is not kept (it can carry script): choose a photo, \
             a JPEG, PNG, WebP or GIF",
        );
    }
    if start.starts_with('<') {
        return invalid(
            "the address gives a web page, not a picture: use the result's url (the picture \
             itself), not its source",
        );
    }
    invalid("it is not a JPEG, PNG, WebP or GIF picture: choose another")
}

/// A picture as it is kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// `raw`, a downloaded file, as the picture Niles keeps: decoded under
/// limits, turned upright, made at most [`MAX_SIDE`] on its long side
/// and written again, as a JPEG or, with transparency, a PNG. An
/// animated GIF or WebP is its first frame.
///
/// Slow for a big photo: run it off the async threads.
pub fn prepare(raw: &[u8]) -> Result<Prepared> {
    let format = sniff(raw).ok_or_else(|| not_a_picture(raw))?;
    let mut reader = ImageReader::with_format(Cursor::new(raw), format.image());
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(unreadable)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS || decoder.total_bytes() > MAX_ALLOC {
        return Err(too_big(width, height));
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut picture = DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    picture.apply_orientation(orientation);
    if picture.width().max(picture.height()) > MAX_SIDE {
        picture = picture.thumbnail(MAX_SIDE, MAX_SIDE);
    }

    let see_through =
        picture.color().has_alpha() && picture.to_rgba8().pixels().any(|p| p.0[3] < u8::MAX);
    let mut bytes = Vec::new();
    let mime = if see_through {
        picture
            .to_rgba8()
            .write_with_encoder(PngEncoder::new(&mut bytes))
            .map_err(unreadable)?;
        "image/png"
    } else {
        picture
            .to_rgb8()
            .write_with_encoder(JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY))
            .map_err(unreadable)?;
        "image/jpeg"
    };
    Ok(Prepared {
        mime,
        bytes,
        width: picture.width(),
        height: picture.height(),
    })
}

fn too_big(width: u32, height: u32) -> Error {
    invalid(format!(
        "it is {width} × {height} pixels, too big to open: choose a smaller picture"
    ))
}

fn unreadable(e: ImageError) -> Error {
    match e {
        ImageError::Limits(_) => invalid("it is too big to open: choose a smaller picture"),
        e => invalid(format!(
            "it could not be read as a picture ({e}): choose another"
        )),
    }
}

/// A picture an artifact keeps, as it is listed: everything but its
/// bytes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Image {
    pub id: i64,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    /// Its size as kept, in bytes.
    pub size: usize,
    /// What it shows, for someone who cannot see it.
    pub alt: String,
    /// Who took or owns it.
    pub credit: Option<String>,
    /// The page it is from, an https address.
    pub source: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Image {
    /// How content names it: `niles-image:12`.
    pub fn reference(&self) -> String {
        format!("{SCHEME}{}", self.id)
    }
}

/// A picture to keep, checked: [`prepare`]d, with its words.
#[derive(Debug, Clone, PartialEq)]
pub struct NewImage {
    pub picture: Prepared,
    pub alt: String,
    pub credit: Option<String>,
    pub source: Option<String>,
}

impl NewImage {
    /// The words checked: `alt` one line and never empty, `credit` one
    /// line, `source` an https address of a named host.
    pub fn new(
        picture: Prepared,
        alt: &str,
        credit: Option<&str>,
        source: Option<&str>,
    ) -> Result<Self> {
        let alt = one_line(alt, "alt", MAX_ALT)?.ok_or_else(|| {
            invalid("alt is what the picture shows, for someone who cannot see it, and it is empty")
        })?;
        let credit = credit
            .map(|c| one_line(c, "credit", MAX_CREDIT))
            .transpose()?
            .flatten();
        let source = source.map(https_address).transpose()?.flatten();
        Ok(Self {
            picture,
            alt,
            credit,
            source,
        })
    }
}

/// A kept picture with its bytes, to serve.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageData {
    pub image: Image,
    pub bytes: Vec<u8>,
}

fn one_line(raw: &str, key: &str, max: usize) -> Result<Option<String>> {
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > max {
        return Err(invalid(format!("{key} is longer than {max} characters")));
    }
    Ok((!line.is_empty()).then_some(line))
}

/// An https address of a named host, with no login in it, as the app
/// will link to it; nothing for an empty one.
fn https_address(raw: &str) -> Result<Option<String>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let refused = || {
        invalid(format!(
            "source is the https:// page it is from, not {raw:?}"
        ))
    };
    if raw.len() > MAX_SOURCE {
        return Err(refused());
    }
    let url = url::Url::parse(raw).map_err(|_| refused())?;
    let named = matches!(url.host(), Some(url::Host::Domain(d)) if d.contains('.'));
    if url.scheme() != "https" || !named || !url.username().is_empty() || url.password().is_some() {
        return Err(refused());
    }
    Ok(Some(url.to_string()))
}

/// Whether one more picture of `size` bytes fits beside `count` kept
/// already, that come to `total`.
pub fn check_room(count: usize, total: usize, size: usize) -> Result<()> {
    if count >= MAX_PER_ARTIFACT {
        return Err(invalid(format!(
            "this artifact keeps {MAX_PER_ARTIFACT} pictures already, the most it may: take \
             one away with artifact_remove_image first"
        )));
    }
    if total + size > MAX_BYTES_PER_ARTIFACT {
        return Err(invalid(format!(
            "its pictures would come to {} MB, and an artifact keeps at most {} MB: take one \
             away with artifact_remove_image first",
            (total + size).div_ceil(1_000_000),
            MAX_BYTES_PER_ARTIFACT / 1_000_000
        )));
    }
    Ok(())
}

/// The picture `raw` names, when it is a whole `niles-image:12`.
pub fn parse_reference(raw: &str) -> Option<i64> {
    let digits = raw.trim().strip_prefix(SCHEME)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|id| *id > 0)
}

/// Every picture `content` names, wherever it names it: a trip's
/// `image`, a document's `![…](niles-image:12)`.
pub fn referenced(content: &Value) -> BTreeSet<i64> {
    let mut found = BTreeSet::new();
    collect(content, &mut found);
    found
}

fn collect(value: &Value, found: &mut BTreeSet<i64>) {
    match value {
        Value::String(s) => {
            for (at, _) in s.match_indices(SCHEME) {
                let digits: String = s[at + SCHEME.len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .take(18)
                    .collect();
                if let Some(id) = digits.parse().ok().filter(|id: &i64| *id > 0) {
                    found.insert(id);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect(v, found)),
        Value::Object(fields) => fields.values().for_each(|v| collect(v, found)),
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, Rgba};
    use serde_json::json;

    pub(crate) fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let picture = ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, 120])
        });
        let mut out = Vec::new();
        picture
            .write_with_encoder(JpegEncoder::new_with_quality(&mut out, 90))
            .unwrap();
        out
    }

    fn png(width: u32, height: u32, alpha: u8) -> Vec<u8> {
        let picture = ImageBuffer::from_fn(width, height, |x, _| Rgba([x as u8, 10, 200, alpha]));
        let mut out = Vec::new();
        picture
            .write_with_encoder(PngEncoder::new(&mut out))
            .unwrap();
        out
    }

    /// `jpeg` with an EXIF segment after its start: the orientation
    /// given, and a GPS position written out where anyone reading the
    /// bytes would find it.
    fn with_exif(jpeg: &[u8], orientation: u16) -> Vec<u8> {
        let mut tiff = b"II*\0\x08\0\0\0".to_vec();
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        tiff.extend_from_slice(b"GPS 55.6761N 12.5683E");
        let mut segment = b"Exif\0\0".to_vec();
        segment.extend_from_slice(&tiff);
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((segment.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&segment);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    #[test]
    fn the_four_formats_are_known_by_their_first_bytes() {
        assert_eq!(sniff(&jpeg(4, 4)), Some(Format::Jpeg));
        assert_eq!(sniff(&png(4, 4, 255)), Some(Format::Png));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Format::Webp));
        assert_eq!(sniff(b"GIF89a\x01\0\x01\0"), Some(Format::Gif));
        for other in [
            &b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"[..],
            b"<!doctype html><html>",
            b"BM6\0\0\0",
            b"\0\0\0\x1cftypavif",
            b"",
        ] {
            assert_eq!(sniff(other), None);
        }
    }

    #[test]
    fn an_svg_or_a_web_page_is_refused_saying_what_it_is() {
        let svg = prepare(b"\xef\xbb\xbf  <?xml version=\"1.0\"?><svg onload=\"alert(1)\"/>")
            .unwrap_err()
            .to_string();
        assert!(svg.contains("SVG"), "{svg}");
        let page = prepare(b"<!doctype html><title>Photo</title>")
            .unwrap_err()
            .to_string();
        assert!(page.contains("web page"), "{page}");
        let other = prepare(b"%PDF-1.7").unwrap_err().to_string();
        assert!(other.contains("JPEG, PNG, WebP or GIF"), "{other}");
    }

    #[test]
    fn a_picture_is_written_again_without_its_exif_and_turned_upright() {
        let raw = with_exif(&jpeg(40, 20), 6);
        assert!(raw.windows(4).any(|w| w == b"Exif"));
        let kept = prepare(&raw).unwrap();
        assert_eq!(kept.mime, "image/jpeg");
        assert_eq!(
            (kept.width, kept.height),
            (20, 40),
            "orientation 6 is a quarter turn"
        );
        assert!(!kept.bytes.windows(4).any(|w| w == b"Exif"));
        assert!(!kept.bytes.windows(3).any(|w| w == b"GPS"));
        assert_eq!(sniff(&kept.bytes), Some(Format::Jpeg));
    }

    #[test]
    fn a_big_picture_is_made_small_and_keeps_its_shape() {
        let kept = prepare(&jpeg(3200, 1800)).unwrap();
        assert_eq!((kept.width, kept.height), (1600, 900));
        let small = prepare(&jpeg(300, 200)).unwrap();
        assert_eq!((small.width, small.height), (300, 200));
    }

    #[test]
    fn transparency_is_kept_as_png_and_an_opaque_png_becomes_a_jpeg() {
        let clear = prepare(&png(30, 30, 128)).unwrap();
        assert_eq!(clear.mime, "image/png");
        assert_eq!(sniff(&clear.bytes), Some(Format::Png));
        let solid = prepare(&png(30, 30, 255)).unwrap();
        assert_eq!(solid.mime, "image/jpeg");
    }

    #[test]
    fn a_gif_is_its_first_frame() {
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            for shade in [10u8, 240] {
                let frame =
                    image::Frame::new(ImageBuffer::from_pixel(8, 6, Rgba([shade, 0, 0, 255])));
                encoder.encode_frame(frame).unwrap();
            }
        }
        let kept = prepare(&gif).unwrap();
        assert_eq!((kept.width, kept.height), (8, 6));
        let first = image::load_from_memory(&kept.bytes).unwrap().to_rgb8();
        assert!(
            first.get_pixel(3, 3).0[0] < 60,
            "{:?}",
            first.get_pixel(3, 3)
        );
    }

    #[test]
    fn a_small_file_claiming_to_be_huge_is_not_opened() {
        // A GIF's header says how big it is; this one says 60,000 square,
        // gigabytes once opened, in a file of a few bytes.
        let mut bomb = b"GIF89a".to_vec();
        bomb.extend_from_slice(&60_000u16.to_le_bytes());
        bomb.extend_from_slice(&60_000u16.to_le_bytes());
        bomb.extend_from_slice(&[0, 0, 0]);
        bomb.extend_from_slice(b",\0\0\0\0");
        bomb.extend_from_slice(&60_000u16.to_le_bytes());
        bomb.extend_from_slice(&60_000u16.to_le_bytes());
        bomb.extend_from_slice(&[0, 2, 2, 0x4c, 1, 0, 0x3b]);
        let e = prepare(&bomb).unwrap_err().to_string();
        assert!(e.contains("too big"), "{e}");
    }

    #[test]
    fn many_pixels_within_the_side_limit_are_too_many_still() {
        // 12,000 × 12,000 is inside the side limit and 144 megapixels.
        let mut bomb = b"GIF89a".to_vec();
        bomb.extend_from_slice(&12_000u16.to_le_bytes());
        bomb.extend_from_slice(&12_000u16.to_le_bytes());
        bomb.extend_from_slice(&[0, 0, 0]);
        bomb.extend_from_slice(b",\0\0\0\0");
        bomb.extend_from_slice(&12_000u16.to_le_bytes());
        bomb.extend_from_slice(&12_000u16.to_le_bytes());
        bomb.extend_from_slice(&[0, 2, 2, 0x4c, 1, 0, 0x3b]);
        let e = prepare(&bomb).unwrap_err().to_string();
        assert!(e.contains("too big"), "{e}");
    }

    #[test]
    fn a_broken_picture_is_refused() {
        let mut cut = jpeg(64, 64);
        cut.truncate(40);
        assert!(prepare(&cut).is_err());
    }

    fn prepared() -> Prepared {
        prepare(&jpeg(4, 4)).unwrap()
    }

    #[test]
    fn a_picture_s_words_are_checked() {
        let kept = NewImage::new(
            prepared(),
            "  The Grand Palace\n at dawn ",
            Some(" Photo: J. Doe "),
            Some("https://Commons.Wikimedia.org/wiki/File:x.jpg"),
        )
        .unwrap();
        assert_eq!(kept.alt, "The Grand Palace at dawn");
        assert_eq!(kept.credit.as_deref(), Some("Photo: J. Doe"));
        assert_eq!(
            kept.source.as_deref(),
            Some("https://commons.wikimedia.org/wiki/File:x.jpg")
        );
        assert!(NewImage::new(prepared(), " ", None, None).is_err());
        assert!(NewImage::new(prepared(), &"a".repeat(MAX_ALT + 1), None, None).is_err());
        for bad in [
            "http://x.example/",
            "javascript:alert(1)",
            "https://localhost/",
            "https://u:p@x.example/",
        ] {
            assert!(
                NewImage::new(prepared(), "x", None, Some(bad)).is_err(),
                "{bad}"
            );
        }
        let blank = NewImage::new(prepared(), "x", Some(""), Some(" ")).unwrap();
        assert_eq!((blank.credit, blank.source), (None, None));
    }

    #[test]
    fn an_artifact_keeps_so_many_pictures_and_so_much() {
        assert!(check_room(0, 0, 500_000).is_ok());
        let full = check_room(MAX_PER_ARTIFACT, 1_000, 10)
            .unwrap_err()
            .to_string();
        assert!(full.contains("artifact_remove_image"), "{full}");
        let heavy = check_room(3, MAX_BYTES_PER_ARTIFACT - 100, 101)
            .unwrap_err()
            .to_string();
        assert!(heavy.contains("20 MB"), "{heavy}");
        assert!(check_room(3, MAX_BYTES_PER_ARTIFACT - 100, 100).is_ok());
    }

    #[test]
    fn references_are_found_wherever_content_has_them() {
        let content = json!({
            "markdown": "![Palace](niles-image:12) and ![x](niles-image:3), niles-image:abc",
            "days": [{ "image": "niles-image:7", "items": [{ "image": "niles-image:12" }] }],
            "n": 5,
        });
        assert_eq!(
            referenced(&content).into_iter().collect::<Vec<_>>(),
            vec![3, 7, 12]
        );
        assert_eq!(parse_reference(" niles-image:42 "), Some(42));
        for bad in [
            "niles-image:",
            "niles-image:0",
            "niles-image:4x",
            "image:4",
            "https://x/4",
        ] {
            assert_eq!(parse_reference(bad), None, "{bad}");
        }
    }
}
