//! `[public]` — the address Niles is reached at from the internet.

use crate::error::{Error, Result};
use serde::Deserialize;

/// Where the internet reaches this Niles, if it does: `https://home.example.dk`.
///
/// The one address Niles cannot work out for itself. A service is given
/// a plugin's webhook address before it ever calls, and a request's
/// `Host` header is the caller's to set, so neither can be trusted with
/// it. Unset, plugins have no webhook and connect no accounts; nothing
/// else in Niles reads it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicConfig {
    #[serde(default)]
    pub url: Option<String>,
}

impl PublicConfig {
    /// The address without a trailing slash, when one is set.
    pub fn url(&self) -> Option<String> {
        self.url
            .as_deref()
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty())
    }

    pub fn validate(&self) -> Result<()> {
        let Some(url) = self.url() else {
            return Ok(());
        };
        let refuse = |reason: &str| {
            Err(Error::InvalidSection {
                section: "public",
                reason: format!("url {url:?} {reason}"),
            })
        };
        let Some(rest) = url.strip_prefix("https://") else {
            return refuse("must start with https://: services that call Niles ask for it");
        };
        let host = rest.split('/').next().unwrap_or_default();
        if host.is_empty() {
            return refuse("has no host");
        }
        if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return refuse("has a space in it");
        }
        if url.contains(['?', '#', '@']) {
            return refuse("is an address only: no query, fragment or username");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(url: &str) -> PublicConfig {
        PublicConfig {
            url: Some(url.into()),
        }
    }

    #[test]
    fn unset_or_empty_is_no_address() {
        assert_eq!(PublicConfig::default().url(), None);
        assert_eq!(public("  ").url(), None);
        assert!(public("").validate().is_ok());
    }

    #[test]
    fn an_https_address_is_kept_without_its_trailing_slash() {
        assert_eq!(
            public(" https://home.example.dk/ ").url().as_deref(),
            Some("https://home.example.dk")
        );
        assert!(public("https://home.example.dk").validate().is_ok());
        assert!(public("https://example.dk/niles").validate().is_ok());
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            "http://home.example.dk",
            "home.example.dk",
            "https://",
            "https://home.example.dk/?x=1",
            "https://user@home.example.dk",
            "https://home example.dk",
        ] {
            assert!(public(bad).validate().is_err(), "{bad}");
        }
    }
}
