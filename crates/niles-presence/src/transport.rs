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

    /// PUT a JSON body with a Bearer token.
    ///
    /// Defaulted so the several test doubles in this crate, which only
    /// ever read, do not each have to grow a method they will never
    /// answer.
    async fn put_bearer(&self, _url: &str, _token: &str, _body: String) -> Result<(u16, String)> {
        Err(Error::Parse {
            reason: "this transport cannot write".into(),
        })
    }

    /// DELETE a URL with a Bearer token.
    async fn delete_bearer(&self, _url: &str, _token: &str) -> Result<(u16, String)> {
        Err(Error::Parse {
            reason: "this transport cannot write".into(),
        })
    }
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

    async fn put_bearer(&self, url: &str, token: &str, body: String) -> Result<(u16, String)> {
        let resp = self
            .http
            .put(url)
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        Ok((status, body))
    }

    async fn delete_bearer(&self, url: &str, token: &str) -> Result<(u16, String)> {
        let resp = self.http.delete(url).bearer_auth(token).send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        Ok((status, body))
    }
}
