//! The `http` import: one request, the whole response, every hop checked.
//!
//! The rules are in [`crate::policy`]; this is the I/O around them. Each
//! hop is admitted by host, resolved here, checked by address, and then
//! sent to *that* address — reqwest is told the name resolves to the IP
//! that passed, so a second lookup cannot point it somewhere else. TLS
//! still verifies the certificate against the name. Redirects are
//! followed by hand for the same reason, at most [`MAX_REDIRECTS`].

use crate::policy::HttpPolicy;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

pub const MAX_BODY: usize = 4 * 1024 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: reqwest::Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    NotPermitted(String),
    InvalidRequest(String),
    Timeout,
    /// What the plugin is told, in plain words, and the library's own,
    /// for its log at debug level.
    Connection(String, Option<String>),
    TooLarge,
}

/// A failure to connect, said plainly, with the library's words kept.
fn connection(context: &str, e: &(dyn std::error::Error + 'static)) -> Failure {
    let said = crate::human::said(e, "the server");
    let said = if context.is_empty() {
        said
    } else {
        format!("{context}: {said}")
    };
    Failure::Connection(said, Some(e.to_string()))
}

/// Send `request` as the plugin `user_agent` names, giving up at
/// `deadline` whatever the request's own timeout says.
pub async fn send(
    policy: &HttpPolicy,
    user_agent: &str,
    request: Request,
    deadline: Instant,
) -> Result<Response, Failure> {
    let budget = request
        .timeout
        .unwrap_or(DEFAULT_TIMEOUT)
        .min(deadline.saturating_duration_since(Instant::now()));
    let until = Instant::now() + budget;

    let mut headers = HeaderMap::new();
    for (name, value) in &request.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| Failure::InvalidRequest(format!("{name:?} is not a header name")))?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| Failure::InvalidRequest(format!("the {name} header is not valid")))?;
        headers.append(name, value);
    }
    if !headers.contains_key(USER_AGENT) {
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(user_agent).unwrap_or(HeaderValue::from_static("niles-plugin")),
        );
    }

    let mut url: reqwest::Url = request
        .url
        .parse()
        .map_err(|e| Failure::InvalidRequest(format!("{:?} is not a URL: {e}", request.url)))?;
    let mut method = request.method;
    let mut body = request.body;
    for _hop in 0..=MAX_REDIRECTS {
        let response = hop(policy, &url, &method, &headers, body.clone(), until).await?;
        let status = response.status();
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .map(str::to_string);
        match (status.is_redirection(), location) {
            (true, Some(location)) => {
                let next = url
                    .join(&location)
                    .map_err(|_| Failure::Connection(format!("a redirect to {location:?}"), None))?;
                // What a browser does: a 303, or a 301/302 after a POST,
                // continues as a GET without the body.
                let becomes_get = status == reqwest::StatusCode::SEE_OTHER
                    || (matches!(status.as_u16(), 301 | 302) && method == reqwest::Method::POST);
                if becomes_get {
                    method = reqwest::Method::GET;
                    body = None;
                }
                // Credentials meant for one host are not handed to another.
                if next.host_str() != url.host_str() {
                    headers.remove(reqwest::header::AUTHORIZATION);
                    headers.remove(reqwest::header::COOKIE);
                }
                url = next;
            }
            _ => return read(response, until).await,
        }
    }
    Err(Failure::Connection(
        format!("more than {MAX_REDIRECTS} redirects"),
        None,
    ))
}

async fn hop(
    policy: &HttpPolicy,
    url: &reqwest::Url,
    method: &reqwest::Method,
    headers: &HeaderMap,
    body: Option<Vec<u8>>,
    until: Instant,
) -> Result<reqwest::Response, Failure> {
    let admitted = policy.admit(url).map_err(Failure::NotPermitted)?;
    let port = url.port_or_known_default().unwrap_or(443);
    let literal = match url.host() {
        Some(url::Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
        _ => None,
    };
    let candidates: Vec<IpAddr> = match literal {
        Some(ip) => vec![ip],
        None => {
            let lookup = tokio::net::lookup_host((admitted.host.as_str(), port));
            match tokio::time::timeout(remaining(until)?, lookup).await {
                Err(_) => return Err(Failure::Timeout),
                Ok(Err(e)) => {
                    let context = format!("{} could not be found", admitted.host);
                    return Err(connection(&context, &e));
                }
                Ok(Ok(addrs)) => addrs.map(|a| a.ip()).collect(),
            }
        }
    };
    let mut refusal = None;
    let mut chosen = None;
    for ip in candidates {
        match policy.allows(&admitted, ip) {
            Ok(()) => {
                chosen = Some(ip);
                break;
            }
            Err(why) => {
                refusal.get_or_insert(why);
            }
        }
    }
    let Some(ip) = chosen else {
        return Err(Failure::NotPermitted(
            refusal.unwrap_or_else(|| format!("{} has no address", admitted.host)),
        ));
    };

    let mut client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // A proxy from the environment would make the address check
        // meaningless: the connection would go wherever it points.
        .no_proxy()
        .timeout(remaining(until)?);
    if literal.is_none() {
        client = client.resolve(&admitted.host, SocketAddr::new(ip, port));
    }
    let client = client
        .build()
        .map_err(|e| connection("", &e))?;
    let mut builder = client
        .request(method.clone(), url.clone())
        .headers(headers.clone());
    if let Some(body) = body {
        builder = builder.body(body);
    }
    builder.send().await.map_err(failure)
}

async fn read(mut response: reqwest::Response, until: Instant) -> Result<Response, Failure> {
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            Some((
                name.as_str().to_ascii_lowercase(),
                value.to_str().ok()?.to_string(),
            ))
        })
        .collect();
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err(Failure::TooLarge);
    }
    let mut body = Vec::new();
    loop {
        let chunk = tokio::time::timeout(remaining(until)?, response.chunk())
            .await
            .map_err(|_| Failure::Timeout)?
            .map_err(failure)?;
        let Some(chunk) = chunk else { break };
        if body.len() + chunk.len() > MAX_BODY {
            return Err(Failure::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Response {
        status,
        headers,
        body,
    })
}

fn remaining(until: Instant) -> Result<Duration, Failure> {
    let left = until.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(Failure::Timeout)
    } else {
        Ok(left)
    }
}

fn failure(e: reqwest::Error) -> Failure {
    if e.is_timeout() {
        Failure::Timeout
    } else if e.is_builder() {
        Failure::InvalidRequest(e.to_string())
    } else {
        connection("", &e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Permissions;

    fn get(url: &str) -> Request {
        Request {
            method: reqwest::Method::GET,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: None,
        }
    }

    fn policy() -> HttpPolicy {
        HttpPolicy::new(
            &Permissions {
                http: vec!["api.example.com".into()],
                ..Default::default()
            },
            |_| None,
        )
    }

    async fn refused(url: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        match send(&policy(), "niles-plugin/test/0.1.0", get(url), deadline).await {
            Err(Failure::NotPermitted(why)) => why,
            other => panic!("{url}: expected a refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn undeclared_hosts_are_refused_before_anything_is_sent() {
        assert!(
            refused("https://elsewhere.example.org/")
                .await
                .contains("not in the manifest")
        );
        assert!(refused("http://api.example.com/").await.contains("https"));
        assert!(
            refused("https://127.0.0.1/")
                .await
                .contains("no plugin may reach")
        );
        assert!(
            refused("http://[::1]:8080/")
                .await
                .contains("no plugin may reach")
        );
        assert!(
            refused("http://169.254.169.254/latest/")
                .await
                .contains("no plugin may reach")
        );
        assert!(
            refused("https://192.168.1.10/")
                .await
                .contains("not in the manifest")
        );
    }

    #[tokio::test]
    async fn a_bad_url_or_header_is_an_invalid_request() {
        let deadline = Instant::now() + Duration::from_secs(5);
        let err = send(&policy(), "ua", get("not a url"), deadline).await;
        assert!(matches!(err, Err(Failure::InvalidRequest(_))), "{err:?}");
        let mut bad = get("https://api.example.com/");
        bad.headers.push(("bad header".into(), "x".into()));
        let err = send(&policy(), "ua", bad, deadline).await;
        assert!(matches!(err, Err(Failure::InvalidRequest(_))), "{err:?}");
    }

    #[tokio::test]
    async fn no_time_left_is_a_timeout_without_a_request() {
        let err = send(
            &policy(),
            "ua",
            get("https://api.example.com/"),
            Instant::now(),
        )
        .await;
        assert_eq!(err.unwrap_err(), Failure::Timeout);
    }
}
