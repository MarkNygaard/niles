//! Who is allowed to use the web surface.
//!
//! See ARCHITECTURE.md § Signing in. The short of it: GitHub is the
//! front door, not the boundary — accounts there are free, so "has a
//! GitHub account" tests nothing. This list is the boundary, and it
//! lives in config rather than a table so that adding somebody is an
//! edit in Settings rather than a message, a token and a mail server.
//!
//! Nothing here enforces anything. It is the list; the middleware that
//! reads it is in `niles-api`.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[auth]` section of the config file.
///
/// Absent, or present with an empty `allowed`, means **sign-in is
/// off**. That is what makes the first person able to get in: on a
/// fresh install there is nobody to authenticate as, and a login page
/// nobody can pass is not a safer state, it is a locked cupboard with
/// the house key inside.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Env-var *names*, as everywhere else credentials are referenced.
    /// The values live in the secret store and never pass through the
    /// config UI.
    pub github_client_id_env: Option<String>,
    pub github_client_secret_env: Option<String>,
    /// Signs the session cookie. Must outlive a restart, or every
    /// deploy signs everybody out.
    pub session_secret_env: Option<String>,
    /// The operator's bearer token, for callers that are not browsers.
    /// Not subject to the allowlist, because it is not a person — and
    /// the way back in if the list is ever emptied.
    pub api_token_env: Option<String>,
    #[serde(default)]
    pub allowed: Vec<AllowedPerson>,
}

/// One person, by the address GitHub will vouch for.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AllowedPerson {
    /// Matched against the **verified** address from `/user/emails` —
    /// never the profile's `email` field, which is whatever the account
    /// holder typed.
    pub email: String,
    /// The enrolled speaker this is the same person as, so voice and
    /// web share one identity and one memory. Optional: without it the
    /// account is web-only.
    pub speaker: Option<String>,
    /// The phone this person carries, as the network sees it.
    ///
    /// One address, on the person, so "one phone per person" is true by
    /// construction rather than by rule. Paired from the phone itself —
    /// the app asks the network which client is making the request —
    /// so nobody types a MAC address, and a new phone is re-paired the
    /// same way.
    ///
    /// iOS presents a private address per network, stable for that
    /// network. Which is fine, and is why this is worth re-pairing
    /// rather than treating as permanent.
    #[serde(default)]
    pub device_mac: Option<String>,
}

impl AuthConfig {
    /// Whether sign-in is switched on.
    ///
    /// Both halves are required. Naming a GitHub app with nobody
    /// allowed would refuse everyone; listing people with no GitHub app
    /// would give them nothing to sign in with.
    pub fn is_enabled(&self) -> bool {
        !self.allowed.is_empty() && self.names_a_github_app()
    }

    /// Whether a GitHub app has been *pointed at*, by either route.
    ///
    /// Naming `*_env` variables is one way to say so; storing the
    /// values from Settings is the other, and an install with no config
    /// file has only the second. Deliberately asks whether an app was
    /// nominated rather than whether its values read back — that
    /// distinction is what lets a deployment whose secrets never
    /// arrived be told apart from one correctly waiting for its first
    /// person, and collapsing it would hide exactly the failure
    /// `is_configured` exists to catch.
    fn names_a_github_app(&self) -> bool {
        let named = self.github_client_id_env.is_some() && self.github_client_secret_env.is_some();
        let stored = crate::secrets::get("auth.github_client_id").is_some()
            && crate::secrets::get("auth.github_client_secret").is_some();
        named || stored
    }

    /// Whether the secrets `[auth]` names can actually be read.
    ///
    /// Separate from [`Self::is_enabled`] on purpose, and the
    /// difference is the point. A deployment whose secrets never
    /// reached the process looks *exactly* like one that is correctly
    /// set up and waiting for its first person: both have an empty
    /// allowlist and both report sign-in as off. One of them is open to
    /// anybody who can reach it.
    ///
    /// Reported by `/auth/status` so the two can be told apart from
    /// outside, which is the only place anybody is going to look.
    pub fn is_configured(&self) -> bool {
        self.resolve_client_id().is_ok()
            && self.resolve_client_secret().is_ok()
            && self.resolve_session_secret().is_ok()
    }

    /// The person this verified address belongs to, if any.
    ///
    /// Case-insensitive: addresses are not, and GitHub returns them as
    /// the holder typed them.
    pub fn person_for(&self, email: &str) -> Option<&AllowedPerson> {
        let email = email.trim();
        self.allowed
            .iter()
            .find(|person| person.email.eq_ignore_ascii_case(email))
    }

    /// The GitHub OAuth app's client id, from the environment.
    pub fn resolve_client_id(&self) -> Result<String> {
        self.resolve("github_client_id_env", self.github_client_id_env.as_deref())
    }

    pub fn resolve_client_secret(&self) -> Result<String> {
        self.resolve(
            "github_client_secret_env",
            self.github_client_secret_env.as_deref(),
        )
    }

    /// The key the session cookie is signed with.
    ///
    /// Required rather than generated: a key minted at startup would be
    /// different after every restart, and everybody would be signed out
    /// by each deploy with nothing to explain why.
    pub fn resolve_session_secret(&self) -> Result<String> {
        self.resolve("session_secret_env", self.session_secret_env.as_deref())
    }

    /// The operator's token, or `None` when none is configured.
    ///
    /// Unlike the others this is not an error when absent: a household
    /// that never reads `/logs` from a terminal needs no token, and
    /// demanding one would be a setting for its own sake.
    pub fn resolve_api_token(&self) -> Option<String> {
        let var = self.api_token_env.as_deref().unwrap_or("");
        crate::env::require_secret("auth", "auth.api_token", var).ok()
    }

    /// `session_secret_env` names where the secret *was* kept; the
    /// purpose is `auth.session_secret`, which is what it is saved
    /// under when somebody sets it in the app instead.
    fn resolve(&self, field: &'static str, var: Option<&str>) -> Result<String> {
        let key = format!("auth.{}", field.trim_end_matches("_env"));
        crate::env::require_secret("auth", &key, var.unwrap_or(""))
    }

    pub fn validate(&self) -> Result<()> {
        for (index, person) in self.allowed.iter().enumerate() {
            person.validate(index)?;
        }

        let mut seen: Vec<String> = Vec::with_capacity(self.allowed.len());
        for person in &self.allowed {
            let normalized = person.email.trim().to_ascii_lowercase();
            if seen.contains(&normalized) {
                return Err(Error::InvalidSection {
                    section: "auth",
                    reason: format!("{} is listed twice", person.email.trim()),
                });
            }
            seen.push(normalized);
        }

        // Half-configured is worth naming rather than silently reading
        // as "off", because the symptom — a sign-in button that isn't
        // there — looks the same either way.
        let named = self.github_client_id_env.is_some() || self.github_client_secret_env.is_some();
        if named && (self.github_client_id_env.is_none() || self.github_client_secret_env.is_none())
        {
            return Err(Error::InvalidSection {
                section: "auth",
                reason: "github_client_id_env and github_client_secret_env go together; \
                         set both or neither"
                    .into(),
            });
        }
        Ok(())
    }

    /// Whether replacing this section with `next` would lock everybody
    /// out, and so must be refused.
    ///
    /// Emptying a non-empty list is the one config edit that cannot be
    /// undone from the page that made it: there would be nobody left
    /// who could sign in to put it back. Removing *yourself* while
    /// somebody else remains is fine — that person can undo it.
    ///
    /// The escape hatch, if it happens some other way, is the bearer
    /// token: it is not subject to this list.
    pub fn would_lock_out(&self, next: &AuthConfig) -> bool {
        !self.allowed.is_empty() && next.allowed.is_empty()
    }
}

impl AllowedPerson {
    fn validate(&self, index: usize) -> Result<()> {
        let email = self.email.trim();
        if email.is_empty() {
            return Err(Error::InvalidSection {
                section: "auth",
                reason: format!("entry {index} has no email address"),
            });
        }
        // Not an RFC 5322 parser, and not trying to be. The address is
        // only ever compared against one GitHub hands us, so the check
        // that earns its place is "did somebody paste a username".
        let Some((local, domain)) = email.split_once('@') else {
            return Err(Error::InvalidSection {
                section: "auth",
                reason: format!("{email:?} is not an email address"),
            });
        };
        if local.is_empty() || !domain.contains('.') || domain.starts_with('.') {
            return Err(Error::InvalidSection {
                section: "auth",
                reason: format!("{email:?} is not an email address"),
            });
        }

        if let Some(speaker) = &self.speaker
            && speaker.trim().is_empty()
        {
            return Err(Error::InvalidSection {
                section: "auth",
                reason: format!("{email} has an empty speaker; omit it instead"),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(email: &str) -> AllowedPerson {
        AllowedPerson {
            email: email.into(),
            speaker: None,
            device_mac: None,
        }
    }

    fn configured(people: Vec<AllowedPerson>) -> AuthConfig {
        AuthConfig {
            github_client_id_env: Some("NILES_GITHUB_CLIENT_ID".into()),
            github_client_secret_env: Some("NILES_GITHUB_CLIENT_SECRET".into()),
            session_secret_env: Some("NILES_SESSION_SECRET".into()),
            api_token_env: None,
            allowed: people,
        }
    }

    #[test]
    fn naming_a_secret_is_not_the_same_as_being_able_to_read_it() {
        // The failure this exists for: a deployment that mounts its
        // secrets under names Kubernetes silently drops. Sign-in then
        // reads as "off, waiting for its first person" — which is also
        // what a correct install looks like at that moment, except
        // that one is open to anybody.
        let cfg = AuthConfig {
            github_client_id_env: Some("NILES_TEST_AUTH_ABSENT_ID".into()),
            github_client_secret_env: Some("NILES_TEST_AUTH_ABSENT_SECRET".into()),
            session_secret_env: Some("NILES_TEST_AUTH_ABSENT_SESSION".into()),
            api_token_env: None,
            allowed: vec![],
        };
        assert!(!cfg.is_configured(), "nothing is set, so nothing resolves");
    }

    #[test]
    fn configured_means_every_secret_resolves() {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var("NILES_TEST_AUTH_PRESENT_ID", "id");
            std::env::set_var("NILES_TEST_AUTH_PRESENT_SECRET", "secret");
            std::env::set_var("NILES_TEST_AUTH_PRESENT_SESSION", "session");
        }
        let mut cfg = AuthConfig {
            github_client_id_env: Some("NILES_TEST_AUTH_PRESENT_ID".into()),
            github_client_secret_env: Some("NILES_TEST_AUTH_PRESENT_SECRET".into()),
            session_secret_env: Some("NILES_TEST_AUTH_PRESENT_SESSION".into()),
            api_token_env: None,
            allowed: vec![],
        };
        assert!(cfg.is_configured());

        // One missing is enough to be misconfigured.
        cfg.session_secret_env = Some("NILES_TEST_AUTH_ABSENT_SESSION".into());
        assert!(!cfg.is_configured());
    }

    #[test]
    fn a_section_that_names_nothing_is_not_configured() {
        assert!(!AuthConfig::default().is_configured());
    }

    #[test]
    fn an_empty_list_means_sign_in_is_off() {
        // Otherwise a fresh install has a login page nobody can pass,
        // which is a locked cupboard with the house key inside.
        assert!(!configured(vec![]).is_enabled());
    }

    #[test]
    fn people_without_a_github_app_cannot_sign_in_either() {
        let cfg = AuthConfig {
            allowed: vec![person("someone@example.com")],
            ..Default::default()
        };
        assert!(!cfg.is_enabled());
    }

    #[test]
    fn one_person_and_an_app_is_enough() {
        assert!(configured(vec![person("someone@example.com")]).is_enabled());
    }

    #[test]
    fn an_address_matches_whatever_case_github_returns_it_in() {
        let cfg = configured(vec![person("Someone@Example.com")]);
        assert!(cfg.person_for("someone@example.com").is_some());
        assert!(cfg.person_for("  SOMEONE@EXAMPLE.COM  ").is_some());
        assert!(cfg.person_for("else@example.com").is_none());
    }

    #[test]
    fn a_username_is_not_an_address() {
        // The commonest mistake: pasting a GitHub login instead of the
        // address GitHub verified.
        let cfg = configured(vec![person("marknygaard")]);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn the_same_person_twice_is_refused() {
        let cfg = configured(vec![person("a@example.com"), person("A@Example.com")]);
        let error = cfg.validate().expect_err("should refuse");
        assert!(error.to_string().contains("listed twice"), "{error}");
    }

    #[test]
    fn half_a_github_app_is_refused_rather_than_read_as_off() {
        // The symptom of "off" and of "misconfigured" is the same
        // missing button, so the difference has to be said out loud.
        let cfg = AuthConfig {
            github_client_id_env: Some("NILES_GITHUB_CLIENT_ID".into()),
            allowed: vec![person("a@example.com")],
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn emptying_the_list_would_lock_everybody_out() {
        let now = configured(vec![person("a@example.com")]);
        assert!(now.would_lock_out(&configured(vec![])));
    }

    #[test]
    fn removing_yourself_is_allowed_while_somebody_remains() {
        let now = configured(vec![person("a@example.com"), person("b@example.com")]);
        assert!(!now.would_lock_out(&configured(vec![person("b@example.com")])));
    }

    #[test]
    fn the_first_person_can_always_be_added() {
        // Nobody is locked out by going from nobody to somebody.
        let now = configured(vec![]);
        assert!(!now.would_lock_out(&configured(vec![person("a@example.com")])));
    }

    #[test]
    fn a_speaker_is_optional_but_not_blank() {
        let mut cfg = configured(vec![person("a@example.com")]);
        assert!(cfg.validate().is_ok());
        cfg.allowed[0].speaker = Some("  ".into());
        assert!(cfg.validate().is_err());
        cfg.allowed[0].speaker = Some("mark".into());
        assert!(cfg.validate().is_ok());
    }
}
