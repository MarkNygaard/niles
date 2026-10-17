//! Files on a chat message: what is taken, how it is checked, and
//! how one is served back.

use crate::chat::{AttachmentKind, Failure, Message, Upload, failed};
use crate::state::AppState;
use axum::extract::{FromRequest, Multipart, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use niles_artifacts::images;

/// Five is enough for a set of photos and keeps one message inside
/// what an assistant can look at in one run.
pub const MAX_ATTACHMENTS: usize = 5;
/// A photo off a phone is a few megabytes; a scanned PDF can be ten.
pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
/// The whole multipart body: every file at its largest, and room for
/// the message and the part headers.
pub const MAX_CHAT_BODY: usize = MAX_ATTACHMENTS * MAX_ATTACHMENT_BYTES + 1024 * 1024;

/// What a file may be served as: what Niles itself writes them as.
const SERVED_TYPES: [&str; 3] = ["image/jpeg", "image/png", "application/pdf"];

/// What the first bytes say a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sniffed {
    Image(images::Format),
    Pdf,
    Heic,
    Other,
}

pub(crate) fn sniff(bytes: &[u8]) -> Sniffed {
    if let Some(format) = images::sniff(bytes) {
        return Sniffed::Image(format);
    }
    if bytes.starts_with(b"%PDF-") {
        return Sniffed::Pdf;
    }
    // ISO-BMFF: a size, then `ftyp`, then the brand. HEIC/HEIF/AVIF
    // share the box; the brand says which. Nothing here decodes any of
    // them, but knowing it is a HEIC photo makes the refusal useful.
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heis" | b"mif1" | b"msf1"
        ) {
            return Sniffed::Heic;
        }
    }
    Sniffed::Other
}

/// What a file is called, as the person will read it back: the last
/// path segment only (a browser may send a whole path), no control
/// characters, and short enough for a chip.
fn display_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last.trim().chars().filter(|c| !c.is_control()).collect();
    let truncated: String = cleaned.chars().take(120).collect();
    if truncated.is_empty() {
        "file".into()
    } else {
        truncated
    }
}

/// "14.2 MB" — one decimal, the unit a person reads a file size in.
fn readable(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// The message and its files out of a `multipart/form-data` body.
pub(crate) async fn read(
    request: axum::extract::Request,
    state: &AppState,
) -> Result<(Message, Vec<Upload>), Failure> {
    let mut multipart = Multipart::from_request(request, state)
        .await
        .map_err(|e| failed(StatusCode::BAD_REQUEST, e.body_text()))?;
    let mut text = String::new();
    let mut conversation_id = None;
    let mut new = false;
    let mut uploads = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| failed(StatusCode::BAD_REQUEST, e.body_text()))?
    {
        // Borrow the name before the field is consumed by `text()` or
        // `bytes()`.
        let part = field.name().map(str::to_owned).unwrap_or_default();
        match part.as_str() {
            "text" => {
                text = field
                    .text()
                    .await
                    .map_err(|e| failed(StatusCode::BAD_REQUEST, e.body_text()))?;
            }
            "conversation_id" => {
                let raw = field
                    .text()
                    .await
                    .map_err(|e| failed(StatusCode::BAD_REQUEST, e.body_text()))?;
                conversation_id = Some(raw.trim().parse::<i64>().map_err(|_| {
                    failed(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "conversation_id must be a number".into(),
                    )
                })?);
            }
            "new" => {
                let raw = field
                    .text()
                    .await
                    .map_err(|e| failed(StatusCode::BAD_REQUEST, e.body_text()))?;
                new = raw == "true";
            }
            "files" => {
                let name = display_name(field.file_name().unwrap_or_default());
                if uploads.len() >= MAX_ATTACHMENTS {
                    return Err(failed(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "Five files at most in one message. Send the rest in another.".into(),
                    ));
                }
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| failed(StatusCode::BAD_REQUEST, e.body_text()))?;
                if bytes.len() > MAX_ATTACHMENT_BYTES {
                    return Err(failed(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!(
                            "{name} is {}, and the most a file can be is 10 MB. \
                             Send a smaller copy.",
                            readable(bytes.len())
                        ),
                    ));
                }
                if bytes.is_empty() {
                    return Err(failed(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("{name} is empty."),
                    ));
                }
                uploads.push(prepare(name, bytes).await?);
            }
            // Anything else is skipped: a field the app does not know
            // yet must not make the whole message fail.
            _ => {}
        }
    }
    Ok((
        Message {
            text,
            conversation_id,
            new,
        },
        uploads,
    ))
}

/// One uploaded part as it is kept: a picture re-written by
/// `niles_artifacts::images`, a PDF kept exactly as it arrived.
async fn prepare(name: String, bytes: axum::body::Bytes) -> Result<Upload, Failure> {
    match sniff(&bytes) {
        Sniffed::Image(_) => {
            let prepared = tokio::task::spawn_blocking(move || images::prepare(&bytes))
                .await
                .map_err(|e| failed(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                .map_err(|e| failed(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
            Ok(Upload {
                kind: AttachmentKind::Image,
                mime: prepared.mime.into(),
                name,
                bytes: prepared.bytes,
                width: Some(prepared.width),
                height: Some(prepared.height),
            })
        }
        Sniffed::Pdf => Ok(Upload {
            kind: AttachmentKind::Pdf,
            mime: "application/pdf".into(),
            name,
            bytes: bytes.to_vec(),
            width: None,
            height: None,
        }),
        Sniffed::Heic => Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "{name} is a HEIC photo, which Niles cannot read yet. On an iPhone, \
                 Settings → Camera → Formats → Most Compatible makes new photos JPEG, \
                 or share this one as a JPEG."
            ),
        )),
        Sniffed::Other => Err(failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{name} is not a JPEG, PNG, WebP, GIF or PDF."),
        )),
    }
}

/// `GET /chat/conversations/{id}/attachments/{a}`
///
/// One of the caller's files, as Niles keeps it. Its id is never
/// reused for other bytes, so the browser keeps it as long as it
/// likes; the headers keep it inert wherever it is opened: not
/// sniffed, nothing in it run or loaded inside Niles's own page, and
/// not to be shown by another site.
pub async fn serve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((conversation, id)): Path<(i64, i64)>,
) -> Result<Response, Failure> {
    let chat = crate::chat::chat(&state)?;
    let (who, speaker) = crate::chat::caller(&state, &headers);
    let served = chat
        .attachment(&who, speaker.as_deref(), conversation, id)
        .await
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, "no such attachment".into()))?;
    let mime = SERVED_TYPES
        .into_iter()
        .find(|t| *t == served.mime)
        .ok_or_else(|| failed(StatusCode::BAD_GATEWAY, "not a file Niles serves".into()))?;
    let mut response = served.bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    // `inline` plus the sandbox policy above is what lets a PDF open
    // in its own tab (or download, where the viewer is not under
    // `sandbox`) while never being renderable inside Niles's page.
    let disposition = format!(
        "inline; filename*=UTF-8''{}",
        urlencoding::encode(&served.name)
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition).unwrap_or_else(|_| HeaderValue::from_static("inline")),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_reads_the_first_bytes() {
        assert_eq!(
            sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Sniffed::Image(images::Format::Jpeg)
        );
        assert_eq!(
            sniff(b"\x89PNG\r\n\x1a\nrest"),
            Sniffed::Image(images::Format::Png)
        );
        assert_eq!(sniff(b"%PDF-1.7\n%abc"), Sniffed::Pdf);
        assert_eq!(sniff(b"<html><body>"), Sniffed::Other);
        assert_eq!(sniff(b""), Sniffed::Other);
        // RIFF without WEBP is not WebP.
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00AVIF"), Sniffed::Other);
    }

    #[test]
    fn sniff_knows_a_heic_box() {
        // ISO-BMFF: size, `ftyp`, brand `heic`, then the minor version.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&24u32.to_be_bytes());
        bytes.extend_from_slice(b"ftypheic");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"heicmif1");
        assert_eq!(sniff(&bytes), Sniffed::Heic);
        // AVIF shares the box shape and is not taken either, but it is
        // not called HEIC.
        let mut avif = Vec::new();
        avif.extend_from_slice(&24u32.to_be_bytes());
        avif.extend_from_slice(b"ftypavif");
        avif.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(sniff(&avif), Sniffed::Other);
    }

    #[test]
    fn a_pdf_named_png_is_a_pdf() {
        // The name is not evidence; the bytes are.
        assert_eq!(sniff(b"%PDF-1.4\n"), Sniffed::Pdf);
    }

    #[test]
    fn display_name_keeps_the_last_segment_only() {
        assert_eq!(display_name("C:\\Users\\x\\photo.jpg"), "photo.jpg");
        assert_eq!(display_name("../../etc/passwd"), "passwd");
        assert_eq!(display_name("holiday photo.png"), "holiday photo.png");
        assert_eq!(display_name(""), "file");
        assert_eq!(display_name("///"), "file");
    }

    #[test]
    fn display_name_drops_control_characters() {
        assert_eq!(display_name("a\u{0}b\u{1f}c.pdf"), "abc.pdf");
        assert_eq!(display_name("   "), "file");
    }

    #[test]
    fn display_name_caps_the_length() {
        let long = "a".repeat(300);
        assert_eq!(display_name(&long).len(), 120);
        // Cut on a char boundary, not mid-way through one.
        let wide = "ø".repeat(200);
        assert!(display_name(&wide).is_char_boundary(120));
        assert_eq!(display_name(&wide).chars().count(), 120);
    }

    #[test]
    fn readable_sizes() {
        assert_eq!(readable(512), "512 B");
        assert_eq!(readable(2048), "2.0 KB");
        assert_eq!(readable(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(readable(14_900_000), "14.2 MB");
    }
}
