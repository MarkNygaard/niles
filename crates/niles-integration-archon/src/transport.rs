//! Archon transport trait + production HTTP implementation.

use crate::error::{Error, Result};
use async_trait::async_trait;
use std::time::Duration;

/// Abstract transport for Archon HTTP requests.
#[async_trait]
pub trait ArchonTransport: Send + Sync {
    /// Perform an HTTP GET and return the response body as a string.
    async fn get(&self, url: &str) -> Result<String>;
    /// Perform an HTTP POST with a JSON body and return the response body as a string.
    async fn post(&self, url: &str, body: &str) -> Result<String>;
}

/// Production transport using `reqwest`.
#[derive(Debug)]
pub struct HttpTransport {
    http: reqwest::Client,
    user_agent: String,
}

impl HttpTransport {
    pub fn new(user_agent: impl Into<String>, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest TLS init");
        Self {
            http,
            user_agent: user_agent.into(),
        }
    }
}

#[async_trait]
impl ArchonTransport for HttpTransport {
    async fn get(&self, url: &str) -> Result<String> {
        let resp = self
            .http
            .get(url)
            .header("User-Agent", &self.user_agent)
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

    async fn post(&self, url: &str, body: &str) -> Result<String> {
        let resp = self
            .http
            .post(url)
            .header("User-Agent", &self.user_agent)
            .header("Content-Type", "application/json")
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
