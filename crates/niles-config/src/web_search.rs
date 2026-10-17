//! Web search configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

fn default_timeout_seconds() -> u64 {
    15
}

fn default_num_results() -> u8 {
    5
}

/// `[web_search]` section of the config file: the house's SearXNG.
///
/// Optional, and set up as an add-on in the app as often as here. With
/// no address, Niles's own `web_search` and `image_search` are not
/// offered, and Claude searches with its own built-in search instead.
/// Read live: changing or removing the address takes hold with the next
/// search, without a restart.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchConfig {
    /// Where the SearXNG instance answers searches, usually ending in
    /// `/search`. Empty is the same as absent: removing the add-on in
    /// the app writes it empty, so an address a ConfigMap still names
    /// is set aside rather than coming back.
    pub base_url: Option<String>,
    /// Request timeout in seconds. Default: 15.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// Default number of results to return. Default: 5.
    #[serde(default = "default_num_results")]
    pub default_num_results: u8,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            timeout_seconds: default_timeout_seconds(),
            default_num_results: default_num_results(),
        }
    }
}

impl WebSearchConfig {
    /// The SearXNG address, when there is one.
    pub fn address(&self) -> Option<&str> {
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(url) = self.address()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
        {
            return Err(Error::InvalidSection {
                section: "web_search",
                reason: format!("base_url '{}' must start with http:// or https://", url),
            });
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > 600 {
            return Err(Error::InvalidSection {
                section: "web_search",
                reason: "timeout_seconds must be between 1 and 600".into(),
            });
        }
        if self.default_num_results == 0 || self.default_num_results > 20 {
            return Err(Error::InvalidSection {
                section: "web_search",
                reason: "default_num_results must be between 1 and 20".into(),
            });
        }
        Ok(())
    }
}
