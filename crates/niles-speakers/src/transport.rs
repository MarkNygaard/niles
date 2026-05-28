//! Sonos transport trait + production HTTP implementation.

use crate::error::{Error, Result};
use async_trait::async_trait;
use std::time::Duration;

#[async_trait]
pub trait SonosTransport: Send + Sync {
    async fn send_action(
        &self,
        endpoint: &str,
        soap_action: &str,
        soap_body: &str,
    ) -> Result<String>;
}

/// Production transport using `reqwest` with a 10-second timeout.
pub struct HttpTransport {
    http: reqwest::Client,
}

impl HttpTransport {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest TLS init");
        Self { http }
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SonosTransport for HttpTransport {
    async fn send_action(
        &self,
        endpoint: &str,
        soap_action: &str,
        soap_body: &str,
    ) -> Result<String> {
        let resp = self
            .http
            .post(endpoint)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("SOAPAction", format!("\"{soap_action}\""))
            .body(soap_body.to_string())
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            const MAX_ERR_BODY: usize = 2048;
            let preview = &body[..body.len().min(MAX_ERR_BODY)];

            let code = extract_tag(preview, "faultcode").unwrap_or_else(|| status.to_string());
            let reason = extract_tag(preview, "faultstring").unwrap_or_else(|| preview.to_string());

            return Err(Error::SoapFault { code, reason });
        }

        Ok(body)
    }
}

/// Small string-slicing helper for fault extraction.
pub(crate) fn extract_tag(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)?;
    Some(body[start..start + end].to_string())
}
