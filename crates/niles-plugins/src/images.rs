//! The pictures on a plugin's pages, fetched by Niles and served to the
//! app, so the viewer's browser never contacts the plugin's hosts: no
//! cookies, no referrer, no tracking pixel.
//!
//! An `https` image on a page is drawn from `GET /plugins/{id}/image?src=`.
//! Niles checks the URL against the plugin's manifest ([`admit`]): the
//! `ui` permission, https, a host its `http` permission declares, no SVG.
//! It then fetches it through the same [`crate::http::send`] a plugin's
//! own requests use, with a policy holding only the `http` hosts, so
//! every hop's host is checked again and a name that resolves to a
//! private or loopback address is refused, as it would be for the
//! plugin. The bytes must be PNG, JPEG, WebP or GIF by their first bytes
//! ([`sniff`]), whatever the `Content-Type` claimed, and at most
//! [`crate::http::MAX_BODY`]. Answers are kept for [`CACHE_FOR`], at most
//! [`CACHE_BYTES`] across every plugin, so ten phones on one page fetch
//! each picture once.
//!
//! A `data:` image needs none of this: it was checked when the tree was,
//! and the app draws it as it is.

use crate::manifest::{Manifest, Permissions};
use crate::policy::HttpPolicy;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a fetched image is served without asking again.
pub const CACHE_FOR: Duration = Duration::from_secs(10 * 60);
/// The most image bytes kept at once, across every plugin.
pub const CACHE_BYTES: usize = 32 * 1024 * 1024;
/// How long one fetch may take, redirects included.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A picture, ready to serve.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// `image/png`, `image/jpeg`, `image/webp` or `image/gif`, as the
    /// bytes say rather than the server.
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Why an image is not served.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageError {
    /// The URL is not one the plugin's pages may show.
    NotAllowed(String),
    /// No such plugin running.
    NotFound(String),
    /// The host did not answer with a picture.
    Upstream(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::NotAllowed(s) | ImageError::NotFound(s) | ImageError::Upstream(s) => {
                f.write_str(s)
            }
        }
    }
}

/// The URL `src` as one of `manifest`'s pages may show it, or why not.
pub fn admit(manifest: &Manifest, src: &str) -> Result<reqwest::Url, String> {
    if !manifest.permissions.ui {
        return Err(format!("{} has no pages", manifest.name));
    }
    let url: reqwest::Url = src.parse().map_err(|_| format!("{src:?} is not a URL"))?;
    if url.scheme() != "https" {
        return Err("an image is fetched over https only".into());
    }
    // Credentials in the URL would be sent by Niles, on the plugin's
    // say-so, to wherever they are for.
    if !url.username().is_empty() || url.password().is_some() {
        return Err("an image URL carries no user name or password".into());
    }
    if url.path().to_ascii_lowercase().ends_with(".svg") {
        return Err("SVG images are not allowed".into());
    }
    let declared = internet_only(&manifest.permissions);
    match url.host_str() {
        Some(host) if declared.declares_host(host) => Ok(url),
        _ => Err(format!(
            "{} is not a host {}'s http permission declares",
            url.host_str().unwrap_or("the URL"),
            manifest.name
        )),
    }
}

/// The manifest's `http` hosts and nothing on the local network: a page's
/// picture comes from the internet, never from a device in the house.
fn internet_only(permissions: &Permissions) -> HttpPolicy {
    HttpPolicy::new(
        &Permissions {
            http: permissions.http.clone(),
            ..Default::default()
        },
        |_| None,
    )
}

/// What `bytes` are by their first bytes: PNG, JPEG, WebP or GIF, or
/// `None` for anything else, SVG included.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The images fetched lately, oldest first.
#[derive(Default)]
pub struct Images {
    kept: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Kept>,
    bytes: usize,
}

struct Kept {
    plugin: String,
    src: String,
    at: Instant,
    image: Image,
}

impl Cache {
    fn get(&mut self, plugin: &str, src: &str, now: Instant) -> Option<Image> {
        self.expire(now);
        self.entries
            .iter()
            .find(|k| k.plugin == plugin && k.src == src)
            .map(|k| k.image.clone())
    }

    fn put(&mut self, plugin: &str, src: &str, image: Image, now: Instant) {
        self.expire(now);
        self.drop_where(|k| k.plugin == plugin && k.src == src);
        let size = image.bytes.len();
        if size > CACHE_BYTES {
            return;
        }
        while self.bytes + size > CACHE_BYTES {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= oldest.image.bytes.len();
        }
        self.bytes += size;
        self.entries.push_back(Kept {
            plugin: plugin.to_string(),
            src: src.to_string(),
            at: now,
            image,
        });
    }

    fn expire(&mut self, now: Instant) {
        self.drop_where(|k| now.duration_since(k.at) >= CACHE_FOR);
    }

    fn drop_where(&mut self, gone: impl Fn(&Kept) -> bool) {
        let mut bytes = 0;
        self.entries.retain(|k| {
            let keep = !gone(k);
            if keep {
                bytes += k.image.bytes.len();
            }
            keep
        });
        self.bytes = bytes;
    }
}

impl Images {
    /// The image at `src` for one of `manifest`'s pages: from the cache,
    /// or fetched now.
    pub async fn get(&self, manifest: &Manifest, src: &str) -> Result<Image, ImageError> {
        let url = admit(manifest, src).map_err(ImageError::NotAllowed)?;
        let id = &manifest.id;
        if let Some(hit) = self.lock().get(id, src, Instant::now()) {
            return Ok(hit);
        }
        let request = crate::http::Request {
            method: reqwest::Method::GET,
            url: url.to_string(),
            headers: vec![(
                "accept".into(),
                "image/png,image/jpeg,image/webp,image/gif".into(),
            )],
            body: None,
            timeout: Some(TIMEOUT),
        };
        let user_agent = format!("niles-plugin/{id}/{}", manifest.version);
        let policy = internet_only(&manifest.permissions);
        let response = crate::http::send(&policy, &user_agent, request, Instant::now() + TIMEOUT)
            .await
            .map_err(|failure| {
                use crate::http::Failure;
                ImageError::Upstream(match failure {
                    Failure::NotPermitted(why) | Failure::InvalidRequest(why) => why,
                    Failure::Connection(why, _) => format!("the image could not be fetched: {why}"),
                    Failure::Timeout => "the image took too long".into(),
                    Failure::TooLarge => format!(
                        "the image is over {} MiB",
                        crate::http::MAX_BODY / (1024 * 1024)
                    ),
                })
            })?;
        if !(200..300).contains(&response.status) {
            return Err(ImageError::Upstream(format!(
                "the image's host answered {}",
                response.status
            )));
        }
        let mime = sniff(&response.body).ok_or_else(|| {
            ImageError::Upstream("that is not a PNG, JPEG, WebP or GIF image".into())
        })?;
        let image = Image {
            mime,
            bytes: response.body,
        };
        self.lock().put(id, src, image.clone(), Instant::now());
        Ok(image)
    }

    /// Forget a removed plugin's images.
    pub fn forget(&self, plugin: &str) {
        self.lock().drop_where(|k| k.plugin == plugin);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(ui: bool) -> Manifest {
        Manifest::parse(&format!(
            r#"
id = "cams"
name = "Cams"
version = "0.1.0"
description = "x"
authors = ["a"]
min_niles = "0.2.0"
interface = "0.1"

[permissions]
http = ["img.example.com", "*.cdn.example.net"]
local_network = ["192.168.1.20"]
ui = {ui}
"#
        ))
        .unwrap()
    }

    #[test]
    fn an_image_comes_from_a_declared_host_over_https() {
        let m = manifest(true);
        assert!(admit(&m, "https://img.example.com/now.png").is_ok());
        assert!(admit(&m, "https://a.cdn.example.net/x.jpg?size=2").is_ok());
        assert!(admit(&m, "http://img.example.com/now.png").is_err());
        assert!(admit(&m, "https://elsewhere.com/now.png").is_err());
    }

    #[test]
    fn the_local_network_permission_is_no_way_to_a_picture() {
        let err = admit(&manifest(true), "https://192.168.1.20/snapshot.jpg").unwrap_err();
        assert!(err.contains("not a host"), "{err}");
    }

    #[test]
    fn svg_credentials_and_pageless_plugins_are_refused() {
        let m = manifest(true);
        assert!(admit(&m, "https://img.example.com/logo.SVG").is_err());
        assert!(admit(&m, "https://jane:pw@img.example.com/a.png").is_err());
        assert!(admit(&m, "data:image/png;base64,AAAA").is_err());
        assert!(admit(&manifest(false), "https://img.example.com/a.png").is_err());
    }

    #[test]
    fn the_bytes_say_what_an_image_is() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"GIF89a..."), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
        assert_eq!(sniff(b"<html>"), None);
        assert_eq!(sniff(b""), None);
    }

    fn image(size: usize) -> Image {
        Image {
            mime: "image/png",
            bytes: vec![0; size],
        }
    }

    #[test]
    fn a_kept_image_is_served_until_it_is_ten_minutes_old() {
        let mut cache = Cache::default();
        let now = Instant::now();
        cache.put("cams", "https://img.example.com/a.png", image(10), now);
        assert!(
            cache
                .get("cams", "https://img.example.com/a.png", now)
                .is_some()
        );
        assert!(
            cache
                .get("other", "https://img.example.com/a.png", now)
                .is_none(),
            "kept per plugin"
        );
        assert!(
            cache
                .get("cams", "https://img.example.com/a.png", now + CACHE_FOR)
                .is_none()
        );
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn past_the_cap_the_oldest_go() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let third = CACHE_BYTES / 3;
        for n in 0..4 {
            cache.put(
                "cams",
                &format!("https://img.example.com/{n}.png"),
                image(third),
                now,
            );
        }
        assert!(cache.bytes <= CACHE_BYTES);
        assert!(
            cache
                .get("cams", "https://img.example.com/0.png", now)
                .is_none()
        );
        assert!(
            cache
                .get("cams", "https://img.example.com/3.png", now)
                .is_some()
        );
        cache.put(
            "cams",
            "https://img.example.com/huge.png",
            image(CACHE_BYTES + 1),
            now,
        );
        assert!(
            cache
                .get("cams", "https://img.example.com/huge.png", now)
                .is_none(),
            "too big to keep at all"
        );
    }
}
