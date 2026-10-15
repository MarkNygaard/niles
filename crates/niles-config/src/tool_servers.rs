//! `[tool_servers.<id>]` — remote MCP servers whose tools Niles offers
//! the model. See "Remote tool servers (MCP)" in ARCHITECTURE.md.
//!
//! Each server's key is a secret, `tool_servers.<id>.api_key`, never a
//! value here.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where Linear's own MCP server is.
pub const LINEAR_URL: &str = "https://mcp.linear.app/mcp";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolServerConfig {
    /// The Streamable HTTP endpoint.
    pub url: String,
    /// What the app calls it. Empty means the id.
    #[serde(default)]
    pub name: String,
    /// An environment variable holding the key, for a deployment that
    /// sets it there rather than in the app.
    #[serde(default)]
    pub api_key_env: String,
    /// Words a request has to mention for this server's tools to be sent
    /// with it. Empty means the id and the name.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// What the model is told about using these tools, beside them: the
    /// household's own conventions, like where a coding task goes.
    #[serde(default)]
    pub notes: String,
    /// Each tool's setting, by the server's name for it. A tool not
    /// listed is on.
    #[serde(default)]
    pub tools: BTreeMap<String, ToolMode>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

/// What the model may do with one tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolMode {
    #[default]
    On,
    /// Called, and held for a person's yes before it runs.
    Ask,
    Off,
}

fn default_timeout_seconds() -> u64 {
    30
}

impl ToolServerConfig {
    pub fn display_name<'a>(&'a self, id: &'a str) -> &'a str {
        if self.name.trim().is_empty() {
            id
        } else {
            self.name.trim()
        }
    }

    /// The keywords as matched: lowercased, and the id and name when
    /// none were given.
    pub fn keywords_for(&self, id: &str) -> Vec<String> {
        let mut words: Vec<String> = if self.keywords.is_empty() {
            vec![id.to_string(), self.display_name(id).to_string()]
        } else {
            self.keywords.clone()
        };
        words = words
            .into_iter()
            .map(|w| w.trim().to_lowercase())
            .filter(|w| !w.is_empty())
            .collect();
        words.sort();
        words.dedup();
        words
    }

    pub fn mode(&self, tool: &str) -> ToolMode {
        self.tools.get(tool).copied().unwrap_or_default()
    }

    /// The key, from the named variable or the secret store. `None` when
    /// neither has one; a server that wants no key can be reached
    /// without.
    pub fn api_key(&self, id: &str) -> Option<String> {
        crate::env::require_secret("tool_servers", &secret_key(id), &self.api_key_env).ok()
    }
}

/// The secret a server's key is kept under.
pub fn secret_key(id: &str) -> String {
    format!("tool_servers.{id}.api_key")
}

/// The server a secret key belongs to, if it is one of these.
pub fn server_of_secret(key: &str) -> Option<&str> {
    key.strip_prefix("tool_servers.")?.strip_suffix(".api_key")
}

/// An id is what prefixes every tool name (`linear__list_issues`), so it
/// keeps to what every model provider accepts in a function name, and
/// has no `__` of its own to be confused with the separator.
pub fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !id.contains("__")
        && !id.ends_with('_')
}

pub fn validate(servers: &BTreeMap<String, ToolServerConfig>) -> Result<()> {
    for (id, server) in servers {
        let invalid = |reason: String| Error::InvalidSection {
            section: "tool_servers",
            reason,
        };
        if !valid_id(id) {
            return Err(invalid(format!(
                "{id:?}: an id is 1 to 32 of a-z, 0-9 and single underscores"
            )));
        }
        let url = server.url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() < 10 {
            return Err(invalid(format!("{id}: url must be an http(s) address")));
        }
        if server.timeout_seconds == 0 || server.timeout_seconds > 120 {
            return Err(invalid(format!(
                "{id}: timeout_seconds must be between 1 and 120"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(toml: &str) -> ToolServerConfig {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn a_tool_not_listed_is_on() {
        let s = server(
            "url = \"https://mcp.linear.app/mcp\"\ntools = { delete_issue = \"off\", create_issue = \"ask\" }\n",
        );
        assert_eq!(s.mode("list_issues"), ToolMode::On);
        assert_eq!(s.mode("delete_issue"), ToolMode::Off);
        assert_eq!(s.mode("create_issue"), ToolMode::Ask);
    }

    #[test]
    fn keywords_default_to_the_id_and_name() {
        let s = server("url = \"https://example.com/mcp\"\nname = \"Sentry\"\n");
        assert_eq!(s.keywords_for("sentry"), ["sentry"]);
        let s = server("url = \"https://example.com/mcp\"\nkeywords = [\"Issue\", \" task \"]\n");
        assert_eq!(s.keywords_for("linear"), ["issue", "task"]);
    }

    #[test]
    fn an_id_cannot_hold_the_separator() {
        assert!(valid_id("linear"));
        assert!(valid_id("my_tracker2"));
        assert!(!valid_id("a__b"));
        assert!(!valid_id("Linear"));
        assert!(!valid_id("_x"));
        assert!(!valid_id("x_"));
        assert!(!valid_id(""));
    }

    #[test]
    fn refuses_an_address_that_is_not_http() {
        let mut servers = BTreeMap::new();
        servers.insert("linear".to_string(), server("url = \"mcp.linear.app\"\n"));
        assert!(validate(&servers).is_err());
        servers.insert(
            "linear".to_string(),
            server(&format!("url = \"{LINEAR_URL}\"\n")),
        );
        assert!(validate(&servers).is_ok());
    }

    #[test]
    fn finds_the_server_a_key_belongs_to() {
        assert_eq!(
            server_of_secret("tool_servers.linear.api_key"),
            Some("linear")
        );
        assert_eq!(server_of_secret("integrations.nemlig.password"), None);
        assert_eq!(secret_key("linear"), "tool_servers.linear.api_key");
    }
}
