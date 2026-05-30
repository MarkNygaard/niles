//! Weather transport trait + production HTTP implementation.

use crate::error::{Error, Result};
use async_trait::async_trait;
use std::time::Duration;

/// Abstract transport for fetching weather data via HTTP GET.
#[async_trait]
pub trait WeatherTransport: Send + Sync {
    /// Perform an HTTP GET and return the response body as a string.
    async fn get(&self, url: &str) -> Result<String>;
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
impl WeatherTransport for HttpTransport {
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
}
