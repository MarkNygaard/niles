//! Which build this is: the version, the commit and the build time.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::fmt;

/// A git commit id: exactly 40 lowercase ASCII hex digits. A newtype
/// because it is shown, linked and compared, never edited.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Commit(String);

impl Commit {
    /// `text` trimmed, lowercased, accepted only as 40 ASCII hex
    /// digits: a full sha, the way `image.yml` passes `github.sha`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(Self(text.to_lowercase()))
        } else {
            None
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first seven characters, the way a commit is named in prose
    /// and logs.
    pub fn short(&self) -> &str {
        &self.0[..7]
    }
}

impl fmt::Display for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The build running now, baked in by `image.yml`: the crate version,
/// and the commit and time it was built from. A build without them —
/// `cargo run`, a local `docker build` — is a dev build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Build {
    version: String,
    commit: Option<Commit>,
    built_at: Option<DateTime<Utc>>,
}

impl Build {
    /// The build these parts describe. An empty or whitespace part is
    /// absent; a malformed one makes this a dev build, loudly: a typo
    /// in the workflow's build arg would otherwise read as "your
    /// install is ancient".
    pub fn new(version: &str, commit: Option<&str>, built_at: Option<&str>) -> Self {
        let commit = commit.and_then(|value| {
            match (value.trim().is_empty(), Commit::parse(value)) {
                (true, _) => None,
                (false, Some(commit)) => Some(commit),
                (false, None) => {
                    tracing::warn!(
                        "[update] NILES_GIT_SHA {value:?} isn't a commit id, so this is a dev build"
                    );
                    None
                }
            }
        });
        let built_at = built_at.and_then(|value| {
            if value.trim().is_empty() {
                return None;
            }
            match DateTime::parse_from_rfc3339(value) {
                Ok(at) => Some(at.with_timezone(&Utc)),
                Err(_) => {
                    tracing::warn!(
                        "[update] NILES_BUILD_TIME {value:?} isn't RFC 3339, so it is ignored"
                    );
                    None
                }
            }
        });
        Self {
            version: version.to_string(),
            commit,
            built_at,
        }
    }

    /// This binary's build. cargo rebuilds the crate when either
    /// variable changes, because rustc records every `option_env!` read
    /// as an environment dependency.
    pub fn current() -> &'static Build {
        static CURRENT: std::sync::LazyLock<Build> = std::sync::LazyLock::new(|| {
            Build::new(
                env!("CARGO_PKG_VERSION"),
                option_env!("NILES_GIT_SHA"),
                option_env!("NILES_BUILD_TIME"),
            )
        });
        &CURRENT
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn commit(&self) -> Option<&Commit> {
        self.commit.as_ref()
    }

    pub fn built_at(&self) -> Option<DateTime<Utc>> {
        self.built_at
    }
}

impl fmt::Display for Build {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.commit {
            Some(commit) => {
                write!(f, "Niles {}, commit {}", self.version, commit.short())?;
                if let Some(at) = self.built_at {
                    write!(f, ", built {} UTC", at.format("%Y-%m-%d %H:%M"))?;
                }
                Ok(())
            }
            None => write!(f, "Niles {}, a dev build", self.version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn parse_accepts_a_full_sha_lowercased() {
        let upper = SHA.to_uppercase();
        let commit = Commit::parse(&upper).expect("40 hex digits parse");
        assert_eq!(commit.as_str(), SHA);
        assert_eq!(commit.short(), "0123456");
    }

    #[test]
    fn parse_refuses_anything_else() {
        for bad in [
            "",
            "abc",
            &SHA[..39],
            &format!("{SHA}0"),
            "g".repeat(40).as_str(),
        ] {
            assert!(Commit::parse(bad).is_none(), "{bad:?} is not a commit id");
        }
    }

    #[test]
    fn missing_empty_and_garbage_parts_make_a_dev_build() {
        for (commit, built_at) in [
            (None, None),
            (Some(""), Some("")),
            (Some("   "), None),
            (Some("not-a-sha"), None),
        ] {
            let build = Build::new("0.6.0", commit, built_at);
            assert!(build.commit().is_none(), "{commit:?}");
            assert!(build.built_at().is_none(), "{built_at:?}");
        }
    }

    #[test]
    fn an_rfc3339_time_with_milliseconds_parses() {
        let build = Build::new("0.6.0", Some(SHA), Some("2026-11-06T10:15:00.123Z"));
        assert_eq!(
            build
                .built_at()
                .map(|at| at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
            Some("2026-11-06T10:15:00.123Z".to_string())
        );
        assert_eq!(build.commit().map(Commit::as_str), Some(SHA));
    }

    #[test]
    fn an_invalid_time_is_ignored() {
        let build = Build::new("0.6.0", Some(SHA), Some("yesterday"));
        assert!(build.built_at().is_none());
    }

    #[test]
    fn display_names_both_kinds() {
        let released = Build::new("0.6.0", Some(SHA), Some("2026-11-06T10:15:00Z"));
        assert_eq!(
            released.to_string(),
            "Niles 0.6.0, commit 0123456, built 2026-11-06 10:15 UTC"
        );
        let dev = Build::new("0.6.0", None, None);
        assert_eq!(dev.to_string(), "Niles 0.6.0, a dev build");
        let no_time = Build::new("0.6.0", Some(SHA), None);
        assert_eq!(no_time.to_string(), "Niles 0.6.0, commit 0123456");
    }

    #[test]
    fn serde_shape_is_the_contract() {
        let dev = Build::new("0.6.0", None, None);
        let json = serde_json::to_value(&dev).expect("serializes");
        assert_eq!(
            json,
            serde_json::json!({
                "version": "0.6.0",
                "commit": null,
                "built_at": null,
            })
        );
    }
}
