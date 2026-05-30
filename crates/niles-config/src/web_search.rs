//! Web search configuration section.

use crate::error::{Error, Result};
use serde::Deserialize;

fn default_timeout_seconds() -> u64 {
    15
}

fn default_num_results() -> u8 {
    5
}

/// `[web_search]` section of the config file.
///
/// Optional. If absent (or `base_url` is `None`), the web search tool
/// is not registered.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchConfig {
    /// Base URL of the SearXNG instance. If `None`, web search is disabled.
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
    pub fn validate(&self) -> Result<()> {
        if let Some(url) = &self.base_url
            && url.trim().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "web_search",
                reason: "base_url must not be empty if present".into(),
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
