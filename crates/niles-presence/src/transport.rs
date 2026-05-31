//! Tado transport trait + production HTTP implementation.

use crate::error::{Error, Result};
use async_trait::async_trait;
use std::time::Duration;

/// Abstract transport for Tado HTTP calls.
///
/// Returns raw `(status, body)` tuples so the caller can interpret
/// 401s (token expiry) without the transport layer short-circuiting.
#[async_trait]
pub trait TadoTransport: Send + Sync {
    /// POST a form-encoded body and return the raw response.
    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, String)>;

    /// GET a URL with a Bearer token and return the raw response.
    async fn get_bearer(&self, url: &str, token: &str) -> Result<(u16, String)>;
}

/// Production transport using `reqwest`.
#[derive(Debug)]
pub struct HttpTadoTransport {
    http: reqwest::Client,
}

impl HttpTadoTransport {
    pub fn new(timeout: Duration) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(Error::Http)?;
        Ok(Self { http })
    }
}

#[async_trait]
impl TadoTransport for HttpTadoTransport {
    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, String)> {
        let resp = self.http.post(url).form(form).send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        Ok((status, body))
    }

    async fn get_bearer(&self, url: &str, token: &str) -> Result<(u16, String)> {
        let resp = self.http.get(url).bearer_auth(token).send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        Ok((status, body))
    }
}
