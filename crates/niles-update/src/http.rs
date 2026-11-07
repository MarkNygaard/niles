//! The HTTP pieces shared by the registry and the compare client.

use crate::error::{Error, Result};
use std::time::Duration;

/// As every Niles client names itself: the image and the commits, so
/// GitHub can tell a Niles from a scraper.
pub(crate) const USER_AGENT: &str = concat!("niles/", env!("CARGO_PKG_VERSION"));
/// How long connecting may take.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long one request may take in all.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// The client every check uses: Niles's user agent, bounded waits. The
/// default redirect policy is kept on purpose: the registry answers a
/// blob request with a 307 to GitHub's CDN, and reqwest drops the
/// `Authorization` header on a cross-host redirect, so the token never
/// leaves ghcr.io.
pub(crate) fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| Error::Client {
            reason: e.to_string(),
        })
}

/// The body of `response`, at most `cap` bytes. Refused by the reported
/// length, then again while the bytes arrive.
pub(crate) async fn read_capped(
    mut response: reqwest::Response,
    cap: usize,
    what: &'static str,
) -> Result<Vec<u8>> {
    if let Some(length) = response.content_length()
        && length > cap as u64
    {
        return Err(Error::TooLarge { what, cap });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| Error::Unreachable {
        what,
        reason: reason(&e),
    })? {
        if bytes.len() + chunk.len() > cap {
            return Err(Error::TooLarge { what, cap });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Why a request failed, as one line. The URL is left out — a failed
/// request's query string can carry a token, and the reason is shown.
fn reason(error: &reqwest::Error) -> String {
    let mut parts = vec![clean(error)];
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(": ")
}

/// `error` written without its URL. reqwest only offers the URL-free
/// form by value (which `Error` has no `Clone` for), so the URL is
/// blanked out of the written form instead.
fn clean(error: &reqwest::Error) -> String {
    let text = error.to_string();
    if let Some(url) = error.url() {
        return text.replace(url.as_str(), "the address");
    }
    text
}

/// A failed send as the check's error: a timeout is named as one, the
/// rest are "couldn't be reached" with the chain of causes.
pub(crate) fn send_error(what: &'static str, error: &reqwest::Error) -> Error {
    if error.is_timeout() {
        return Error::Timeout {
            what,
            after_secs: REQUEST_TIMEOUT.as_secs(),
        };
    }
    Error::Unreachable {
        what,
        reason: reason(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A body over the cap is refused by its reported length, before a
    /// byte is read.
    #[tokio::test]
    async fn read_capped_refuses_by_content_length() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(1024)))
            .mount(&server)
            .await;
        let response = client()
            .expect("the client builds")
            .get(format!("{}/big", server.uri()))
            .send()
            .await
            .expect("the answer arrives");
        let why = read_capped(response, 16, "the body").await.unwrap_err();
        assert!(
            matches!(why, Error::TooLarge { cap: 16, .. }),
            "refused before reading: {why}"
        );
    }

    /// Without a reported length the cap is enforced on the arriving
    /// bytes. Built straight from an `http::Response` because wiremock
    /// always reports a length for the bodies it serves.
    #[tokio::test]
    async fn read_capped_refuses_while_summing_chunks() {
        let raw = http::Response::builder()
            .status(200)
            .body("hello world, this is far too long")
            .expect("the response builds");
        let response = reqwest::Response::from(raw);
        let why = read_capped(response, 16, "the body").await.unwrap_err();
        assert!(
            matches!(why, Error::TooLarge { cap: 16, .. }),
            "refused while summing: {why}"
        );
    }

    #[tokio::test]
    async fn read_capped_keeps_a_body_under_the_cap() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/small"))
            .respond_with(ResponseTemplate::new(200).set_body_string("fits"))
            .mount(&server)
            .await;
        let response = client()
            .expect("the client builds")
            .get(format!("{}/small", server.uri()))
            .send()
            .await
            .expect("the answer arrives");
        let bytes = read_capped(response, 16, "the body").await.expect("fits");
        assert_eq!(bytes, b"fits");
    }

    /// A connection failure maps to "couldn't be reached", and the
    /// reason carries no URL: there is nothing to leak.
    #[tokio::test]
    async fn a_refused_connection_is_unreachable() {
        // A port that was listening a moment ago and isn't now. (A
        // dropped wiremock server hands its port to the pool and can
        // still answer, so this uses a bare listener.)
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds an ephemeral port");
        let address = format!("http://{}", listener.local_addr().expect("the address"));
        drop(listener);
        let error = client()
            .expect("the client builds")
            .get(address)
            .send()
            .await
            .expect_err("nothing listens any more");
        let why = send_error("the stand-in", &error);
        let text = why.to_string();
        assert!(
            text.contains("couldn't be reached"),
            "named as unreachable: {text}"
        );
        assert!(!text.contains("http"), "no URL in the reason: {text}");
    }
}
