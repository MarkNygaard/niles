//! Reading one web page as plain text, for the LLM.
//!
//! Search snippets often hold the answer and sometimes do not: "when do
//! Arsenal play next" came back as five links to fixture lists and no
//! date. This fetches one of them and hands back its words.
//!
//! The model asking for a page is also the model that can switch off the
//! lights, and the address it asks for may have come from a page a
//! stranger wrote. So only the public internet is readable: no localhost,
//! no LAN, no `*.cluster.local` — checked on the name, on every address it
//! resolves to, and again after every redirect, with the connection pinned
//! to the address that was checked so DNS cannot change its answer in
//! between. Without that, a search result could send it to read Niles's
//! own API, or the router's.

use crate::error::{Error, Result};
use reqwest::Url;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// Enough of a page to answer from; more is a page the model will not
/// read to the end anyway, at the cost of every token in it.
const MAX_CHARS: usize = 8_000;
/// Stop downloading past this: a page this size is not an article.
const MAX_BYTES: usize = 3 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// Where it ended up, after redirects.
    pub url: String,
    pub title: Option<String>,
    pub text: String,
    /// Whether the text was cut at [`MAX_CHARS`].
    pub truncated: bool,
}

pub struct PageReader {
    timeout: Duration,
    user_agent: String,
}

impl PageReader {
    pub fn new(timeout: Duration, user_agent: impl Into<String>) -> Self {
        Self {
            timeout,
            user_agent: user_agent.into(),
        }
    }

    pub async fn read(&self, address: &str) -> Result<Page> {
        let mut url =
            Url::parse(address.trim()).map_err(|e| refused(format!("not a web address: {e}")))?;
        for _ in 0..=MAX_REDIRECTS {
            let addr = public_address(&url).await?;
            let host = url.host_str().unwrap_or_default().to_string();
            // A client per hop, pinned to the address just checked.
            let client = reqwest::Client::builder()
                .timeout(self.timeout)
                .redirect(reqwest::redirect::Policy::none())
                .resolve(&host, addr)
                .user_agent(&self.user_agent)
                .build()?;
            let mut resp = client
                .get(url.clone())
                .header(reqwest::header::ACCEPT, "text/html,text/plain;q=0.9")
                .send()
                .await?;

            if resp.status().is_redirection() {
                let next = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .ok_or_else(|| unreadable("a redirect with nowhere to go"))?;
                url = url
                    .join(next)
                    .map_err(|e| unreadable(format!("a redirect to {next:?}: {e}")))?;
                continue;
            }
            if !resp.status().is_success() {
                return Err(Error::BadStatus {
                    status: resp.status().as_u16(),
                    body: String::new(),
                });
            }
            let kind = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|c| c.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            let html = kind.contains("html");
            if !html && !kind.starts_with("text/plain") && !kind.is_empty() {
                return Err(unreadable(format!("not a page of text ({kind})")));
            }

            let mut body = Vec::new();
            while let Some(chunk) = resp.chunk().await? {
                body.extend_from_slice(&chunk);
                if body.len() > MAX_BYTES {
                    body.truncate(MAX_BYTES);
                    break;
                }
            }
            let (title, text) = if html {
                page_text(&body)?
            } else {
                (None, String::from_utf8_lossy(&body).into_owned())
            };
            let (text, truncated) = clip(&tidy(&text), MAX_CHARS);
            return Ok(Page {
                url: url.to_string(),
                title,
                text,
                truncated,
            });
        }
        Err(unreadable("too many redirects"))
    }
}

fn refused(reason: impl Into<String>) -> Error {
    Error::Refused {
        reason: reason.into(),
    }
}

fn unreadable(reason: impl Into<String>) -> Error {
    Error::Unreadable {
        reason: reason.into(),
    }
}

/// The one address this URL may be fetched from, or why it may not.
async fn public_address(url: &Url) -> Result<SocketAddr> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(refused(format!("only web pages, not {}:", url.scheme())));
    }
    let host = url
        .host_str()
        .ok_or_else(|| refused("an address with no host"))?;
    if !public_name(host) {
        return Err(refused(format!("{host} is not on the public internet")));
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = match bare.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((bare, port))
            .await
            .map_err(|e| unreadable(format!("could not look up {host}: {e}")))?
            .collect(),
    };
    // All of them, not the first: a name that resolves to one public and
    // one private address is a name pointing into the house.
    if addrs.is_empty() || addrs.iter().any(|a| !public_ip(a.ip())) {
        return Err(refused(format!("{host} resolves inside the network")));
    }
    Ok(addrs[0])
}

/// Names that are never the public internet, whatever they resolve to.
fn public_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<IpAddr>().is_ok() || host.starts_with('[') {
        return true; // judged on the address itself
    }
    const PRIVATE_SUFFIXES: [&str; 6] = [
        ".local",
        ".localdomain",
        ".internal",
        ".lan",
        ".home.arpa",
        ".cluster.local",
    ];
    host.contains('.')
        && host != "localhost"
        && !host.ends_with(".localhost")
        && !PRIVATE_SUFFIXES.iter().any(|s| host.ends_with(s))
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // carrier-grade NAT
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (18..20).contains(&o[1]))
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80) // link local
        }
    }
}

/// The page's title and its readable text, without the markup.
fn page_text(html: &[u8]) -> Result<(Option<String>, String)> {
    let title = title_of(&String::from_utf8_lossy(html));
    let text = html2text::config::plain()
        .string_from_read(html, 120)
        .map_err(|e| unreadable(format!("could not read the page: {e}")))?;
    Ok((title, text))
}

fn title_of(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let title = html[open_end..close]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        // The entities a title actually carries; BBC's has "&amp;".
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    (!title.is_empty()).then_some(title)
}

/// Blank lines and indentation squeezed out: they are tokens with nothing
/// in them.
fn tidy(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn clip(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((cut, _)) => (text[..cut].to_string(), true),
        None => (text.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_house_is_not_the_internet() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.42.1",
            "172.16.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "1.1.1.1", "2606:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn internal_names_are_refused_before_any_lookup() {
        for host in [
            "localhost",
            "niles.localhost",
            "searxng.productivity.svc.cluster.local",
            "printer.local",
            "nas.lan",
            "router",
            "box.home.arpa",
        ] {
            assert!(!public_name(host), "{host}");
        }
        assert!(public_name("www.bbc.co.uk"));
    }

    #[tokio::test]
    async fn a_page_inside_the_network_is_not_read() {
        let reader = PageReader::new(Duration::from_secs(2), "test");
        for address in [
            "http://127.0.0.1:8080/config",
            "http://192.168.42.1/",
            "http://searxng.productivity.svc.cluster.local:8080/search",
            "file:///etc/passwd",
        ] {
            let err = reader.read(address).await.unwrap_err();
            assert!(matches!(err, Error::Refused { .. }), "{address}: {err}");
        }
    }

    #[test]
    fn a_page_is_its_words() {
        let html = br#"<html><head><title> Arsenal  Scores &amp; Fixtures | BBC Sport </title>
            <script>var tracking = 1;</script><style>.x{color:red}</style></head>
            <body><nav>Home</nav><h1>Fixtures</h1>
            <p>Saturday 10th October</p><p>Arsenal v Leeds United, 12:30</p></body></html>"#;
        let (title, text) = page_text(html).unwrap();
        assert_eq!(
            title.as_deref(),
            Some("Arsenal Scores & Fixtures | BBC Sport")
        );
        let text = tidy(&text);
        assert!(text.contains("Saturday 10th October"), "{text}");
        assert!(text.contains("Arsenal v Leeds United, 12:30"), "{text}");
        assert!(!text.contains("tracking"), "scripts are not words: {text}");
    }

    #[test]
    fn a_long_page_is_cut_where_a_character_ends() {
        let (text, cut) = clip(&"é".repeat(10), 4);
        assert_eq!(text, "éééé");
        assert!(cut);
        assert_eq!(clip("short", 10), ("short".into(), false));
    }
}
