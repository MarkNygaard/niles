//! Sonos transport trait + production HTTP implementation.

use crate::error::{Error, Result};
use async_trait::async_trait;
use std::time::Duration;

/// Abstract transport for sending SOAP actions to a Sonos speaker.
#[async_trait]
pub trait SonosTransport: Send + Sync {
    /// Send a raw SOAP action and return the response body.
    async fn send_action(
        &self,
        endpoint: &str,
        soap_action: &str,
        soap_body: &str,
    ) -> Result<String>;
}

/// Production transport using `reqwest` with a 10-second timeout.
#[derive(Debug)]
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
            // Use str::get so we never split a multi-byte UTF-8 boundary.
            let preview = body.get(..MAX_ERR_BODY).unwrap_or(&body);

            // UPnP faults wrap the real error in <detail><UPnPError><errorCode>…
            // Fall back to the generic SOAP <faultcode> / <faultstring> if absent.
            let code = extract_tag(preview, "errorCode")
                .or_else(|| extract_tag(preview, "faultcode"))
                .unwrap_or_else(|| status.to_string());
            let reason = extract_tag(preview, "errorDescription")
                .or_else(|| extract_tag(preview, "faultstring"))
                .unwrap_or_else(|| preview.to_string());

            return Err(Error::SoapFault { code, reason });
        }

        Ok(body)
    }
}

/// Small string-slicing helper for fault extraction.
pub(crate) fn extract_tag(body: &str, tag: &str) -> Option<String> {
    let open = find_open_tag(body, tag)?;
    let close = find_close_tag(&body[open..], tag)? + open;
    Some(body[open..close].trim().to_string())
}

fn find_open_tag(body: &str, tag: &str) -> Option<usize> {
    let mut i = 0;
    while let Some(rel) = body[i..].find('<') {
        let start = i + rel;
        let bytes = body.as_bytes();
        let mut name_start = start + 1;
        if name_start >= bytes.len() {
            return None;
        }
        match bytes[name_start] {
            b'/' | b'!' | b'?' => {
                i = name_start;
                continue;
            }
            _ => {}
        }

        while name_start < bytes.len() && bytes[name_start].is_ascii_whitespace() {
            name_start += 1;
        }
        if name_start >= bytes.len() {
            return None;
        }

        let mut name_end = name_start;
        while name_end < bytes.len()
            && !bytes[name_end].is_ascii_whitespace()
            && bytes[name_end] != b'>'
            && bytes[name_end] != b'/'
        {
            name_end += 1;
        }
        if name_end <= name_start {
            i = start + 1;
            continue;
        }

        let name = &body[name_start..name_end];
        let local = name.rsplit(':').next().unwrap_or(name);
        if local == tag {
            let gt = body[name_end..].find('>')?;
            return Some(name_end + gt + 1);
        }

        i = name_end;
    }
    None
}

fn find_close_tag(body: &str, tag: &str) -> Option<usize> {
    let mut i = 0;
    while let Some(rel) = body[i..].find("</") {
        let start = i + rel + 2;
        let bytes = body.as_bytes();
        let mut name_end = start;
        while name_end < bytes.len()
            && !bytes[name_end].is_ascii_whitespace()
            && bytes[name_end] != b'>'
        {
            name_end += 1;
        }
        if name_end <= start {
            i = start;
            continue;
        }
        let name = &body[start..name_end];
        let local = name.rsplit(':').next().unwrap_or(name);
        if local == tag {
            return Some(i + rel);
        }
        i = name_end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tag_happy_path() {
        assert_eq!(
            extract_tag("<foo>bar</foo>", "foo"),
            Some("bar".to_string())
        );
    }

    #[test]
    fn extract_tag_missing_open() {
        assert_eq!(extract_tag("no tags here", "foo"), None);
    }

    #[test]
    fn extract_tag_missing_close() {
        assert_eq!(extract_tag("<foo>bar", "foo"), None);
    }

    #[test]
    fn extract_tag_supports_namespaced_open_and_close() {
        assert_eq!(
            extract_tag("<u:foo>bar</u:foo>", "foo"),
            Some("bar".to_string())
        );
    }

    #[test]
    fn extract_tag_supports_attributes() {
        assert_eq!(
            extract_tag(r#"<foo xmlns="urn:test">bar</foo>"#, "foo"),
            Some("bar".to_string())
        );
    }

    #[test]
    fn extract_tag_prefers_upnp_error_detail() {
        let body = r#"<s:Fault>
  <faultcode>s:Client</faultcode>
  <faultstring>UPnPError</faultstring>
  <detail>
    <UPnPError xmlns="urn:schemas-upnp-org:control-1-0">
      <errorCode>401</errorCode>
      <errorDescription>Invalid Action</errorDescription>
    </UPnPError>
  </detail>
</s:Fault>"#;
        assert_eq!(extract_tag(body, "errorCode"), Some("401".to_string()));
        assert_eq!(
            extract_tag(body, "errorDescription"),
            Some("Invalid Action".to_string())
        );
    }
}
