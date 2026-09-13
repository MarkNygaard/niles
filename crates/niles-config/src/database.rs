//! `[database]` — where Niles keeps state that must outlive a restart.
//!
//! Optional. Without it Niles falls back to `[persistence]`'s directory,
//! or to memory, and says so at startup.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[database]` section of the config file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    /// Name of the environment variable holding the connection string —
    /// never the string itself. Config files are mounted from a ConfigMap
    /// and read by the config API; a DSN carries a password, so it lives
    /// in a Secret and arrives as an env var like every other credential.
    pub url_env: String,

    /// Pool size. One long-lived process reading at startup and writing
    /// rarely, so the default is small on purpose — the role's own
    /// connection limit is the backstop, and a low cap turns a connection
    /// leak into Niles's problem rather than the cluster's.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Name of the environment variable holding the key that secrets
    /// typed into the app are sealed with, base64 of 32 bytes.
    ///
    /// It cannot live in the database it protects, and it cannot be set
    /// from the app for the same reason — so with the connection string
    /// it is the whole of what Niles needs before the app can take over.
    /// Absent means secrets stay in environment variables, which is
    /// what every install did before this existed.
    #[serde(default = "default_secret_key_env")]
    pub secret_key_env: String,
}

fn default_secret_key_env() -> String {
    "NILES_SECRET_KEY".into()
}

fn default_max_connections() -> u32 {
    5
}

impl DatabaseConfig {
    pub fn validate(&self) -> Result<()> {
        if self.url_env.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "database",
                reason: "url_env must name the environment variable holding the \
                         connection string"
                    .into(),
            });
        }
        // A DSN in this field is the mistake this check exists for: it
        // would be a password sitting in a ConfigMap, in git, and served
        // by the config API.
        if self.url_env.contains("://") {
            return Err(Error::InvalidSection {
                section: "database",
                reason: format!(
                    "url_env must be the *name* of an environment variable, not a \
                     connection string (got something that looks like a DSN: '{}...')",
                    &self.url_env[..self.url_env.len().min(12)]
                ),
            });
        }
        if self.max_connections == 0 {
            return Err(Error::InvalidSection {
                section: "database",
                reason: "max_connections must be at least 1".into(),
            });
        }
        Ok(())
    }

    /// Read the connection string from the environment.
    pub fn resolve_url(&self) -> Result<String> {
        let value = std::env::var(&self.url_env).map_err(|_| Error::InvalidSection {
            section: "database",
            reason: format!(
                "environment variable {} is not set; it must hold the database \
                 connection string",
                self.url_env
            ),
        })?;
        if value.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "database",
                reason: format!("environment variable {} is empty", self.url_env),
            });
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_str: &str) -> DatabaseConfig {
        toml::from_str(toml_str).expect("parses")
    }

    #[test]
    fn url_env_is_required_and_max_connections_defaults() {
        let cfg = parse(r#"url_env = "NILES_DATABASE_URL""#);
        assert_eq!(cfg.url_env, "NILES_DATABASE_URL");
        assert_eq!(cfg.max_connections, 5);
        cfg.validate().unwrap();
    }

    #[test]
    fn a_dsn_in_url_env_is_rejected() {
        // The failure mode worth catching: a password in a ConfigMap, in
        // git, and served by GET /config.
        let cfg = parse(r#"url_env = "postgres://niles:pw@host/niles""#);
        let err = cfg.validate().expect_err("a DSN must be refused");
        assert!(err.to_string().contains("name"), "{err}");
        assert!(
            !err.to_string().contains("pw@host"),
            "the error must not echo the credential back: {err}"
        );
    }

    #[test]
    fn an_empty_url_env_is_rejected() {
        assert!(parse(r#"url_env = "  ""#).validate().is_err());
    }

    #[test]
    fn a_zero_pool_is_rejected() {
        let cfg = parse("url_env = \"X\"\nmax_connections = 0");
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn resolve_url_says_which_variable_is_missing() {
        let cfg = parse(r#"url_env = "NILES_TEST_DEFINITELY_UNSET""#);
        let err = cfg.resolve_url().expect_err("unset variable");
        assert!(err.to_string().contains("NILES_TEST_DEFINITELY_UNSET"));
    }
}
