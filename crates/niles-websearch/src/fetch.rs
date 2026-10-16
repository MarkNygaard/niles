//! Fetching one file from the public internet for Niles to keep: a
//! picture for an artifact, whose address a model chose.
//!
//! Stricter than reading a page ([`crate::page`]), because what comes
//! back is kept: https only, on its usual port, with no login in the
//! address. The name is checked, then every address it resolves to
//! (see [`crate::address`]), and the connection goes to the address that
//! was checked, so DNS cannot answer differently the second time; no
//! proxy, which would look the name up again itself. Each redirect is
//! checked the same way, three at most. No cookies and no credentials
//! are sent, and no referrer. The whole fetch has a deadline, and the
//! download stops past its size as it streams, whatever the server said
//! the size would be.

use crate::address::{public_ip, public_name};
use crate::error::{Error, Result};
use async_trait::async_trait;
use reqwest::Url;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

const MAX_REDIRECTS: usize = 3;
/// However long the whole fetch may take, a server that does not answer
/// the connection is given up on by then.
const CONNECT_WITHIN: Duration = Duration::from_secs(5);

/// A file as it came, from where it ended up.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    /// Where it was fetched from, after redirects.
    pub url: String,
    pub bytes: Vec<u8>,
    /// What the server said it was, which is not to be believed.
    pub content_type: Option<String>,
}

/// Looks a name up. The system's resolver; a test's own map of names.
#[async_trait]
trait Resolve: Send + Sync {
    async fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>>;
}

struct System;

#[async_trait]
impl Resolve for System {
    async fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host((host, port)).await?.collect())
    }
}

/// Where a fetch may go. Only a test loosens these, to reach a server on
/// this machine over plain http.
#[derive(Clone, Copy)]
struct Rules {
    schemes: &'static [&'static str],
    any_port: bool,
    allowed: fn(IpAddr) -> bool,
}

const PUBLIC: Rules = Rules {
    schemes: &["https"],
    any_port: false,
    allowed: public_ip,
};

pub struct PublicFetcher {
    max_bytes: usize,
    timeout: Duration,
    user_agent: String,
    resolver: Arc<dyn Resolve>,
    rules: Rules,
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

impl PublicFetcher {
    /// At most `max_bytes`, within `timeout` from the first request to
    /// the last byte.
    pub fn new(max_bytes: usize, timeout: Duration, user_agent: impl Into<String>) -> Self {
        Self {
            max_bytes,
            timeout,
            user_agent: user_agent.into(),
            resolver: Arc::new(System),
            rules: PUBLIC,
        }
    }

    /// The file at `address`, asking for `accept`.
    pub async fn fetch(&self, address: &str, accept: &str) -> Result<Fetched> {
        tokio::time::timeout(self.timeout, self.follow(address, accept))
            .await
            .map_err(|_| {
                unreadable(format!(
                    "it did not arrive within {} seconds",
                    self.timeout.as_secs_f32()
                ))
            })?
    }

    async fn follow(&self, address: &str, accept: &str) -> Result<Fetched> {
        let mut url =
            Url::parse(address.trim()).map_err(|e| refused(format!("not a web address: {e}")))?;
        for hop in 0..=MAX_REDIRECTS {
            let addr = self.vetted(&url).await?;
            let mut client = reqwest::Client::builder()
                .connect_timeout(CONNECT_WITHIN.min(self.timeout))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .referer(false)
                .user_agent(&self.user_agent);
            if let Some(url::Host::Domain(host)) = url.host() {
                client = client.resolve(host, addr);
            }
            let mut resp = client
                .build()?
                .get(url.clone())
                .header(reqwest::header::ACCEPT, accept)
                .send()
                .await?;

            if resp.status().is_redirection() {
                if hop == MAX_REDIRECTS {
                    break;
                }
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
            if resp
                .content_length()
                .is_some_and(|n| n > self.max_bytes as u64)
            {
                return Err(self.too_big());
            }
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|c| c.to_str().ok())
                .map(str::to_string);
            let mut bytes = Vec::new();
            while let Some(chunk) = resp.chunk().await? {
                if bytes.len() + chunk.len() > self.max_bytes {
                    return Err(self.too_big());
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(Fetched {
                url: url.to_string(),
                bytes,
                content_type,
            });
        }
        Err(unreadable(format!(
            "it was redirected more than {MAX_REDIRECTS} times"
        )))
    }

    fn too_big(&self) -> Error {
        refused(format!(
            "it is bigger than {} MB",
            self.max_bytes.div_ceil(1024 * 1024)
        ))
    }

    /// The one address `url` may be fetched from, or why it may not.
    async fn vetted(&self, url: &Url) -> Result<SocketAddr> {
        if !self.rules.schemes.contains(&url.scheme()) {
            return Err(refused(format!(
                "only an https:// address, not {}:",
                url.scheme()
            )));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(refused("an address with a login in it"));
        }
        if url.port().is_some() && !self.rules.any_port {
            return Err(refused("an address on a port of its own"));
        }
        let port = url.port_or_known_default().unwrap_or(443);
        let addrs: Vec<SocketAddr> = match url.host() {
            Some(url::Host::Domain(host)) => {
                if !public_name(host) {
                    return Err(refused(format!("{host} is not on the public internet")));
                }
                self.resolver
                    .resolve(host, port)
                    .await
                    .map_err(|e| unreadable(format!("could not look up {host}: {e}")))?
            }
            Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
            Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
            None => return Err(refused("an address with no host")),
        };
        // Every one, not the first: a name that resolves to one public
        // and one private address is a name pointing into the house.
        if addrs.is_empty() || addrs.iter().any(|a| !(self.rules.allowed)(a.ip())) {
            return Err(refused(format!(
                "{} is not on the public internet",
                url.host_str().unwrap_or_default()
            )));
        }
        Ok(addrs[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Names the test chose, to the addresses it chose.
    struct Names(HashMap<String, Vec<IpAddr>>);

    #[async_trait]
    impl Resolve for Names {
        async fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
            Ok(self
                .0
                .get(host)
                .map(|ips| ips.iter().map(|ip| SocketAddr::new(*ip, port)).collect())
                .unwrap_or_default())
        }
    }

    fn names(pairs: &[(&str, &[&str])]) -> Arc<Names> {
        Arc::new(Names(
            pairs
                .iter()
                .map(|(name, ips)| {
                    (
                        name.to_string(),
                        ips.iter().map(|ip| ip.parse().unwrap()).collect(),
                    )
                })
                .collect(),
        ))
    }

    /// The public rules, but with this machine counted as the internet
    /// and plain http allowed: so a server here stands in for a site.
    fn here(resolver: Arc<Names>, max_bytes: usize) -> PublicFetcher {
        PublicFetcher {
            resolver,
            rules: Rules {
                schemes: &["http", "https"],
                any_port: true,
                allowed: |ip| ip.is_loopback() || public_ip(ip),
            },
            ..PublicFetcher::new(max_bytes, Duration::from_secs(5), "niles-test")
        }
    }

    fn site(server: &MockServer, rest: &str) -> String {
        format!("http://pictures.test:{}{rest}", server.address().port())
    }

    fn local() -> Arc<Names> {
        names(&[
            ("pictures.test", &["127.0.0.1"]),
            ("inside.test", &["192.168.1.10"]),
            ("both.test", &["127.0.0.1", "10.0.0.7"]),
        ])
    }

    #[tokio::test]
    async fn a_file_is_fetched_from_the_address_that_was_checked_and_nothing_is_sent_with_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/palace.jpg"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/jpeg")
                    .set_body_bytes(b"\xff\xd8\xff picture".to_vec()),
            )
            .mount(&server)
            .await;
        // pictures.test is in no DNS: it is reached through the address the
        // check resolved it to, or not at all.
        let got = here(local(), 1024)
            .fetch(&site(&server, "/palace.jpg"), "image/*")
            .await
            .unwrap();
        assert_eq!(got.bytes, b"\xff\xd8\xff picture");
        assert_eq!(got.content_type.as_deref(), Some("image/jpeg"));
        let sent = &server.received_requests().await.unwrap()[0];
        for header in ["cookie", "authorization", "referer", "proxy-authorization"] {
            assert!(sent.headers.get(header).is_none(), "{header}");
        }
        assert_eq!(sent.headers.get("accept").unwrap(), "image/*");
    }

    #[tokio::test]
    async fn every_redirect_is_checked_again() {
        let server = MockServer::start().await;
        for (from, to) in [
            ("/to-router", "http://192.168.1.1/admin".to_string()),
            (
                "/to-metadata",
                "http://169.254.169.254/latest/meta-data/".to_string(),
            ),
            ("/to-mapped", "http://[::ffff:10.0.0.1]/".to_string()),
            (
                "/to-inside",
                format!("http://inside.test:{}/x", server.address().port()),
            ),
            (
                "/to-both",
                format!("http://both.test:{}/x", server.address().port()),
            ),
            (
                "/to-cluster",
                "http://niles.default.svc.cluster.local/".to_string(),
            ),
            ("/to-file", "file:///etc/passwd".to_string()),
        ] {
            Mock::given(path(from))
                .respond_with(ResponseTemplate::new(302).insert_header("location", to.as_str()))
                .mount(&server)
                .await;
        }
        let fetcher = here(local(), 1024);
        for from in [
            "/to-router",
            "/to-metadata",
            "/to-mapped",
            "/to-inside",
            "/to-both",
            "/to-cluster",
            "/to-file",
        ] {
            let e = fetcher
                .fetch(&site(&server, from), "*/*")
                .await
                .unwrap_err();
            assert!(matches!(e, Error::Refused { .. }), "{from}: {e}");
        }
    }

    #[tokio::test]
    async fn three_redirects_are_followed_and_a_fourth_is_not() {
        let server = MockServer::start().await;
        let hops = ["/a", "/b", "/c", "/d", "/picture"];
        for pair in hops.windows(2) {
            Mock::given(path(pair[0]))
                .respond_with(ResponseTemplate::new(301).insert_header("location", pair[1]))
                .mount(&server)
                .await;
        }
        Mock::given(path("/picture"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"GIF89a".to_vec()))
            .mount(&server)
            .await;
        let fetcher = here(local(), 1024);
        let got = fetcher.fetch(&site(&server, "/b"), "*/*").await.unwrap();
        assert_eq!(got.bytes, b"GIF89a");
        assert!(got.url.ends_with("/picture"), "{}", got.url);
        let e = fetcher
            .fetch(&site(&server, "/a"), "*/*")
            .await
            .unwrap_err();
        assert!(e.to_string().contains("more than 3"), "{e}");
    }

    #[tokio::test]
    async fn a_file_bigger_than_the_cap_is_refused_by_its_length() {
        let server = MockServer::start().await;
        Mock::given(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 2049]))
            .mount(&server)
            .await;
        Mock::given(path("/fits"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 2048]))
            .mount(&server)
            .await;
        let fetcher = here(local(), 2048);
        let e = fetcher
            .fetch(&site(&server, "/big"), "*/*")
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Refused { .. }), "{e}");
        assert_eq!(
            fetcher
                .fetch(&site(&server, "/fits"), "*/*")
                .await
                .unwrap()
                .bytes
                .len(),
            2048
        );
    }

    #[tokio::test]
    async fn a_file_that_does_not_say_its_size_is_stopped_at_the_cap() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n")
                .await;
            let chunk = format!("400\r\n{}\r\n", "x".repeat(0x400));
            // A megabyte, a kilobyte at a time, for a cap of four.
            for _ in 0..1024 {
                if socket.write_all(chunk.as_bytes()).await.is_err() {
                    return;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        });
        let e = here(local(), 4096)
            .fetch(&format!("http://pictures.test:{port}/stream"), "*/*")
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Refused { .. }), "{e}");
    }

    #[tokio::test]
    async fn a_server_that_does_not_answer_in_time_is_given_up_on() {
        let server = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
            .mount(&server)
            .await;
        let fetcher = PublicFetcher {
            timeout: Duration::from_millis(300),
            ..here(local(), 1024)
        };
        let e = fetcher
            .fetch(&site(&server, "/slow"), "*/*")
            .await
            .unwrap_err();
        assert!(e.to_string().contains("did not arrive"), "{e}");
    }

    #[tokio::test]
    async fn only_an_https_address_of_the_public_internet_is_fetched() {
        let fetcher = PublicFetcher {
            resolver: names(&[
                ("photos.example.com", &["93.184.216.34", "10.1.2.3"]),
                ("rebind.example.com", &["127.0.0.1"]),
                ("v6.example.com", &["fd00::1"]),
            ]),
            ..PublicFetcher::new(1024, Duration::from_secs(2), "niles-test")
        };
        for address in [
            "http://upload.wikimedia.org/a.jpg",
            "ftp://upload.wikimedia.org/a.jpg",
            "file:///etc/passwd",
            "https://user:pw@upload.wikimedia.org/a.jpg",
            "https://upload.wikimedia.org:8443/a.jpg",
            "https://localhost/a.jpg",
            "https://searxng.productivity.svc.cluster.local/a.jpg",
            "https://127.0.0.1/a.jpg",
            "https://0.0.0.0/a.jpg",
            "https://169.254.169.254/latest/meta-data/",
            "https://100.64.1.1/a.jpg",
            "https://[::1]/a.jpg",
            "https://[::ffff:127.0.0.1]/a.jpg",
            "https://[fd00::5]/a.jpg",
            "https://photos.example.com/a.jpg",
            "https://rebind.example.com/a.jpg",
            "https://v6.example.com/a.jpg",
            "not an address",
        ] {
            let e = fetcher.fetch(address, "*/*").await.unwrap_err();
            assert!(matches!(e, Error::Refused { .. }), "{address}: {e}");
        }
    }
}
