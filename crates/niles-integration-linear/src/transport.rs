//! Linear transport trait + production HTTP implementation.

use crate::error::{Error, Result};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, USER_AGENT};
use std::time::Duration;

pub const LINEAR_GRAPHQL_URL: &str = "https://api.linear.app/graphql";

/// Abstract transport for Linear GraphQL requests.
#[async_trait]
pub trait LinearTransport: Send + Sync {
    /// POST a GraphQL query body and return the response as a string.
    async fn post_graphql(&self, body: &str) -> Result<String>;
}

/// Build the default header map for Linear API requests.
///
/// **Trap**: Linear personal API keys use `Authorization: <raw_key>` —
/// NO "Bearer " prefix. This helper is the single place the auth header
/// is built so it can be unit-tested.
pub fn build_default_headers(api_key: &str, user_agent: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, api_key.parse().expect("valid header value"));
    headers.insert(
        CONTENT_TYPE,
        "application/json".parse().expect("valid header value"),
    );
    headers.insert(USER_AGENT, user_agent.parse().expect("valid header value"));
    headers
}

/// Production transport using `reqwest`.
#[derive(Debug)]
pub struct HttpTransport {
    http: reqwest::Client,
}

impl HttpTransport {
    pub fn new(api_key: &str, user_agent: &str, timeout: Duration) -> Self {
        let headers = build_default_headers(api_key, user_agent);
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .default_headers(headers)
            .build()
            .expect("reqwest TLS init");
        Self { http }
    }
}

#[async_trait]
impl LinearTransport for HttpTransport {
    async fn post_graphql(&self, body: &str) -> Result<String> {
        let resp = self
            .http
            .post(LINEAR_GRAPHQL_URL)
            .body(body.to_owned())
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            return Err(Error::BadStatus {
                status: status.as_u16(),
                body,
            });
        }

        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_default_headers_raw_key_no_bearer() {
        let headers = build_default_headers("lin_api_test", "ua");
        let auth = headers.get(AUTHORIZATION).unwrap().to_str().unwrap();
        assert_eq!(auth, "lin_api_test");
        assert!(!auth.starts_with("Bearer"));
    }
}
