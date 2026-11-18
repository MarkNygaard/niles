//! Each saved login's site icon, for Vault's list (NIL-85).
//!
//! Niles fetches it once through the public-only fetcher: the site's
//! `/favicon.ico`, else the icon its home page names. Only a picture
//! Niles recognises by its bytes is kept, never SVG, which could carry a
//! script, and never more than [`MAX_ICON`]. What was found, or that
//! nothing was, is kept in memory for a day, so a list of logins costs
//! the sites one fetch each a day at most.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

/// The largest icon kept.
pub const MAX_ICON: usize = 100 * 1024;
/// The most of a home page read to find its icon: the `<head>` is near
/// the top.
const MAX_PAGE: usize = 1024 * 1024;
/// How long an icon, or the lack of one, is kept.
const KEPT_FOR: Duration = Duration::from_secs(24 * 60 * 60);
/// How long one fetch may take.
const FETCH_WITHIN: Duration = Duration::from_secs(8);

/// A site's icon, as Niles serves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub bytes: Arc<[u8]>,
    pub mime: &'static str,
}

/// What fetches from the public internet. The public-only fetcher in
/// the house; a test's own pages in tests.
#[async_trait]
pub trait Fetch: Send + Sync {
    /// At most `max` bytes of `address`, or nothing.
    async fn fetch(&self, address: &str, accept: &str, max: usize) -> Option<Vec<u8>>;
}

/// The public-only fetcher: https on its usual port, public addresses
/// only, every redirect checked.
pub struct Public;

#[async_trait]
impl Fetch for Public {
    async fn fetch(&self, address: &str, accept: &str, max: usize) -> Option<Vec<u8>> {
        niles_websearch::PublicFetcher::new(max, FETCH_WITHIN, "Niles")
            .fetch(address, accept)
            .await
            .ok()
            .map(|fetched| fetched.bytes)
    }
}

/// The icons Niles found, by origin.
pub struct Icons {
    fetch: Box<dyn Fetch>,
    kept: Mutex<HashMap<String, (Instant, Option<Icon>)>>,
}

impl Default for Icons {
    fn default() -> Self {
        Self::new(Box::new(Public))
    }
}

impl Icons {
    pub fn new(fetch: Box<dyn Fetch>) -> Self {
        Self {
            fetch,
            kept: Mutex::new(HashMap::new()),
        }
    }

    /// The icon of `origin`, an `https://host` origin.
    pub async fn of(&self, origin: &str) -> Option<Icon> {
        let now = Instant::now();
        if let Some((at, icon)) = self
            .kept
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(origin)
            && now.duration_since(*at) < KEPT_FOR
        {
            return icon.clone();
        }
        let icon = self.find(origin).await;
        self.kept
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(origin.to_string(), (now, icon.clone()));
        icon
    }

    async fn find(&self, origin: &str) -> Option<Icon> {
        let base = Url::parse(origin).ok()?;
        if base.scheme() != "https" {
            return None;
        }
        if let Some(icon) = self.picture(base.join("/favicon.ico").ok()?.as_str()).await {
            return Some(icon);
        }
        let page = self
            .fetch
            .fetch(base.as_str(), "text/html", MAX_PAGE)
            .await?;
        let href = linked_icon(&String::from_utf8_lossy(&page))?;
        let address = base.join(&href).ok()?;
        if address.scheme() != "https" {
            return None;
        }
        self.picture(address.as_str()).await
    }

    async fn picture(&self, address: &str) -> Option<Icon> {
        let bytes = self.fetch.fetch(address, "image/*", MAX_ICON).await?;
        let mime = picture_type(&bytes)?;
        Some(Icon {
            bytes: bytes.into(),
            mime,
        })
    }
}

/// What a picture is, by its first bytes: PNG, ICO, GIF, JPEG or WebP.
/// Anything else, SVG among it, is no icon Niles serves.
pub fn picture_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() > MAX_ICON {
        return None;
    }
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0, 0, 1, 0, ..] => Some("image/x-icon"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        _ => None,
    }
}

/// The icon a page's `<link>` names: an apple-touch-icon first, which is
/// larger, else an icon or shortcut icon. Its `href` as written.
pub fn linked_icon(html: &str) -> Option<String> {
    let mut icon = None;
    let mut touch = None;
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<link") {
        let start = from + at;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        let tag = &html[start..end];
        let tag_lower = &lower[start..end];
        from = end;
        let Some(rel) = attribute(tag, tag_lower, "rel") else {
            continue;
        };
        let Some(href) = attribute(tag, tag_lower, "href") else {
            continue;
        };
        let rel = rel.to_ascii_lowercase();
        let words: Vec<&str> = rel.split_whitespace().collect();
        if words.contains(&"apple-touch-icon") && touch.is_none() {
            touch = Some(href);
        } else if words.contains(&"icon") && icon.is_none() {
            icon = Some(href);
        }
    }
    touch.or(icon).filter(|href| !href.trim().is_empty())
}

/// A tag's attribute, quoted or not, from its lower-cased twin's offsets.
fn attribute(tag: &str, tag_lower: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = tag_lower[from..].find(name) {
        let start = from + at;
        from = start + name.len();
        let before = tag_lower[..start].chars().last();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = tag_lower[from..].trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let value_at = tag.len() - rest.len() + 1;
        let value = tag[value_at..].trim_start();
        return Some(match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next()?.to_string(),
            _ => value.split(char::is_whitespace).next()?.to_string(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    /// Pages by address; counts every fetch.
    #[derive(Default)]
    struct Pages {
        pages: HashMap<String, Vec<u8>>,
        fetched: AtomicUsize,
    }

    #[async_trait]
    impl Fetch for Pages {
        async fn fetch(&self, address: &str, _accept: &str, max: usize) -> Option<Vec<u8>> {
            self.fetched.fetch_add(1, Ordering::SeqCst);
            self.pages.get(address).filter(|p| p.len() <= max).cloned()
        }
    }

    fn icons(pages: &[(&str, &[u8])]) -> (Icons, Arc<Pages>) {
        let shared = Arc::new(Pages {
            pages: pages
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_vec()))
                .collect(),
            ..Pages::default()
        });
        struct Shared(Arc<Pages>);
        #[async_trait]
        impl Fetch for Shared {
            async fn fetch(&self, address: &str, accept: &str, max: usize) -> Option<Vec<u8>> {
                self.0.fetch(address, accept, max).await
            }
        }
        (Icons::new(Box::new(Shared(shared.clone()))), shared)
    }

    #[tokio::test]
    async fn the_favicon_comes_first_and_is_kept_for_a_day() {
        let (icons, pages) = icons(&[("https://shop.example/favicon.ico", PNG)]);
        let icon = icons.of("https://shop.example").await.expect("an icon");
        assert_eq!(icon.mime, "image/png");
        assert!(icons.of("https://shop.example").await.is_some());
        assert_eq!(pages.fetched.load(Ordering::SeqCst), 1, "fetched once");
    }

    #[tokio::test]
    async fn else_the_home_pages_linked_icon_is_used() {
        let html = br#"<html><head><link rel="stylesheet" href="/a.css"><LINK REL="icon" HREF="/static/icon.png"><link rel='apple-touch-icon' href='/touch.png'></head></html>"#;
        let (icons, _) = icons(&[
            ("https://shop.example/", html),
            ("https://shop.example/touch.png", PNG),
        ]);
        let icon = icons.of("https://shop.example").await.expect("an icon");
        assert_eq!(icon.mime, "image/png");
    }

    #[tokio::test]
    async fn no_icon_is_kept_as_none_and_not_fetched_again() {
        let (icons, pages) = icons(&[]);
        assert!(icons.of("https://shop.example").await.is_none());
        assert!(icons.of("https://shop.example").await.is_none());
        assert_eq!(
            pages.fetched.load(Ordering::SeqCst),
            2,
            "favicon and home page, once"
        );
    }

    #[tokio::test]
    async fn an_svg_or_an_oversized_picture_is_no_icon() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        let (icons, _) = icons(&[("https://shop.example/favicon.ico", svg)]);
        assert!(icons.of("https://shop.example").await.is_none());
        let mut big = PNG.to_vec();
        big.resize(MAX_ICON + 1, 0);
        assert_eq!(picture_type(&big), None);
    }

    #[test]
    fn an_unquoted_link_reads_too() {
        assert_eq!(
            linked_icon("<link rel=icon href=/i.png>"),
            Some("/i.png".into())
        );
        assert_eq!(linked_icon("<p>no icon</p>"), None);
    }

    #[tokio::test]
    async fn a_plain_http_origin_is_not_fetched() {
        let (icons, pages) = icons(&[("http://shop.example/favicon.ico", PNG)]);
        assert!(icons.of("http://shop.example").await.is_none());
        assert_eq!(pages.fetched.load(Ordering::SeqCst), 0);
    }
}
