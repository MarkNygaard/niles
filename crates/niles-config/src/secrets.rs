//! Secrets that did not come from the environment.
//!
//! Config files name an *environment variable*; the value never appears
//! in them. That works when somebody is writing YAML, and not at all
//! when the answer is supposed to be typed into the app — a running
//! process cannot give itself a new environment.
//!
//! So there are two places a secret can come from, and this is the one
//! that is not `std::env`. Values are put here at startup, read out of
//! the database they were saved to, and replaced when somebody changes
//! one. Nothing here knows how they were stored; see `niles-db` for
//! that, which is also where they are encrypted.
//!
//! Held in memory on purpose. Secrets are few, small, and read on paths
//! that cannot be async — [`require_env`](crate::env) is called from
//! sync config code — so the alternative is a database round trip at
//! every use, or making half the config API async to avoid one.

use std::collections::HashMap;
use std::sync::RwLock;

static STORED: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Replace everything held, which is what a reload is.
///
/// Whole-map replacement rather than per-key edits: a secret that was
/// deleted has to actually disappear, and merging would leave it behind
/// until a restart.
pub fn load(values: HashMap<String, String>) {
    *STORED.write().unwrap_or_else(|e| e.into_inner()) = Some(values);
}

/// The stored value for a purpose, if there is one.
///
/// Keys are the config path the secret belongs to — `mqtt.password`,
/// `stt.api_key` — rather than an environment variable name. The point
/// is to say what the secret is *for*, so it survives somebody renaming
/// the variable they used to keep it in.
pub fn get(key: &str) -> Option<String> {
    STORED
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()?
        .get(key)
        .filter(|v| !v.trim().is_empty())
        .cloned()
}

/// Whether a store has been loaded at all.
///
/// Distinguishes "no secret saved" from "nothing has been loaded yet",
/// which are different failures: the first is somebody who has not
/// finished setting Niles up, the second is a bug.
pub fn is_loaded() -> bool {
    STORED.read().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Where a credential is actually coming from.
///
/// Resolved in the same order [`require_secret`](crate::env) uses, so
/// what a page reports and what Niles reads cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Source {
    /// An environment variable the config names. Niles can read it and
    /// cannot change it.
    Environment,
    /// Saved in Niles's own store, which is the only kind it can
    /// replace or clear.
    Stored,
    /// Nowhere. Whatever needs it does not work.
    Unset,
}

impl crate::Config {
    /// The environment variable this purpose is configured to read, if
    /// the config names one.
    ///
    /// The mapping lives here rather than in the API because it is the
    /// config that decides it — and because a second copy of this list
    /// is a second thing to forget to update.
    pub fn secret_env_var(&self, key: &str) -> Option<String> {
        let named = match key {
            "mqtt.username" => Some(self.mqtt.username_env.clone()),
            "mqtt.password" => Some(self.mqtt.password_env.clone()),
            "stt.api_key" => Some(self.stt.api_key_env.clone()),
            "llm.api_key" => Some(self.llm.api_key_env.clone()),
            "integrations.linear.api_key" => self
                .integrations
                .linear
                .as_ref()
                .map(|l| l.api_key_env.clone()),
            "auth.session_secret" => self.auth.session_secret_env.clone(),
            "auth.github_client_secret" => self.auth.github_client_secret_env.clone(),
            "auth.api_token" => self.auth.api_token_env.clone(),
            _ => None,
        }?;
        (!named.trim().is_empty()).then_some(named)
    }

    /// What a credential is used against, when that is knowable.
    ///
    /// "Speech-to-text API key" says what it is for and not who has to
    /// have issued it — and the answer is decided by `base_url`, which
    /// is configurable. Naming the provider in the label would be a
    /// second copy of that setting, wrong the moment somebody points it
    /// somewhere else. The host is read from the same config the
    /// request goes to, so it cannot disagree.
    pub fn secret_hint(&self, key: &str) -> Option<String> {
        match key {
            "mqtt.username" | "mqtt.password" => (!self.mqtt.host.trim().is_empty())
                .then(|| format!("{}:{}", self.mqtt.host, self.mqtt.port)),
            "stt.api_key" => host_of(&self.stt.base_url),
            "llm.api_key" => host_of(&self.llm.base_url),
            "integrations.linear.api_key" => Some("api.linear.app".into()),
            "auth.github_client_id" | "auth.github_client_secret" => Some("github.com".into()),
            // Niles's own, used against nothing.
            _ => None,
        }
    }

    /// Where a credential comes from right now.
    ///
    /// Environment first, matching resolution: a variable that is set
    /// wins, and reporting it as merely "stored" — or worse, as unset —
    /// would invite somebody to type a value here and silently move
    /// where the credential lives.
    pub fn secret_source(&self, key: &str) -> Source {
        if let Some(var) = self.secret_env_var(key)
            && std::env::var(&var).is_ok_and(|v| !v.trim().is_empty())
        {
            return Source::Environment;
        }
        if get(key).is_some() {
            return Source::Stored;
        }
        // A role pointing at a provider is served by that provider's
        // key. Without this the page reports speech-to-text as having
        // no key while it is happily using Groq's — the same mistake as
        // reading the `*_env` name instead of the value, one level
        // further along.
        if let Some(provider) = self.provider_for(key)
            && get(&provider).is_some()
        {
            return Source::Stored;
        }
        Source::Unset
    }

    /// The provider key a role's credential falls through to, if the
    /// role names one.
    fn provider_for(&self, key: &str) -> Option<String> {
        let named = match key {
            "stt.api_key" => self.stt.provider.as_deref(),
            "llm.api_key" => self.llm.provider.as_deref(),
            _ => None,
        }?;
        crate::providers::find(&self.providers, named).map(|p| p.secret_key())
    }
}

/// The host out of a base URL, without pulling in a URL parser for one
/// label. Anything unparseable is simply not shown.
fn host_of(base_url: &str) -> Option<String> {
    let rest = base_url.split_once("://")?.1;
    let host = rest.split(['/', '?']).next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// Serialises tests that write the process-wide store.
///
/// The store is global by design — `require_env` is called from sync
/// config code — which makes it shared between tests in a crate. Two
/// of them loading different maps at once is how a test about speech
/// ended up asserting on a broker password.
#[cfg(test)]
pub(crate) static TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    // The store is process-wide, and tests in a crate share a process.
    // These use keys nothing else asks for, so loading one does not
    // hand a value to a test about something else — which is exactly
    // what happened the first time these were written.

    #[test]
    fn an_unloaded_store_has_nothing_and_says_so() {
        // Not `load`ing here on purpose: these tests share a process, so
        // this one only asserts what `get` does with a miss.
        assert_eq!(get("nothing.saved.here"), None);
    }

    #[test]
    fn a_stored_secret_is_found_by_purpose() {
        load(HashMap::from([(
            "test.only.password".into(),
            "hunter2".into(),
        )]));
        assert!(is_loaded());
        assert_eq!(get("test.only.password").as_deref(), Some("hunter2"));
    }

    #[test]
    fn an_empty_value_is_not_a_secret() {
        // A placeholder nobody filled in must not read as configured,
        // for the same reason an empty env var does not: the process
        // starts happily and fails at the first call instead.
        load(HashMap::from([("test.only.blank".into(), "   ".into())]));
        assert_eq!(get("test.only.blank"), None);
    }

    #[test]
    fn an_environment_variable_is_reported_as_the_source() {
        // The bug this exists to prevent: the first version only ever
        // looked in the store, so every credential fed by an env var
        // read as unset — and the page offered to overwrite working
        // credentials, which is exactly the guard it was meant to be.
        unsafe { std::env::set_var("NILES_TEST_SOURCE_KEY", "from-the-env") };
        let cfg = crate::Config::load_from_str(
            "[mqtt]
password_env = \"NILES_TEST_SOURCE_KEY\"
",
        )
        .expect("valid");
        assert_eq!(cfg.secret_source("mqtt.password"), Source::Environment);
    }

    #[test]
    fn a_named_variable_that_is_not_set_is_not_a_source() {
        let cfg = crate::Config::load_from_str(
            "[mqtt]
password_env = \"NILES_TEST_DEFINITELY_UNSET_XYZ\"
",
        )
        .expect("valid");
        assert_eq!(cfg.secret_source("mqtt.password"), Source::Unset);
    }

    #[test]
    fn the_environment_wins_over_the_store() {
        // Same order the reader uses. Reporting "stored" while the
        // reader takes the env var would make the page describe a
        // credential Niles is not using.
        unsafe { std::env::set_var("NILES_TEST_SOURCE_WINS", "from-the-env") };
        load(HashMap::from([("stt.api_key".into(), "stored".into())]));
        let cfg = crate::Config::load_from_str(
            "[stt]
api_key_env = \"NILES_TEST_SOURCE_WINS\"
",
        )
        .expect("valid");
        assert_eq!(cfg.secret_source("stt.api_key"), Source::Environment);
        load(HashMap::new());
    }

    #[test]
    fn a_key_says_where_it_is_used() {
        // The label cannot: "LLM API key" is true of every provider,
        // and which one it must come from is decided by `base_url`.
        let cfg = crate::Config::load_from_str("").expect("valid");
        assert_eq!(
            cfg.secret_hint("llm.api_key").as_deref(),
            Some("api.groq.com")
        );
        assert_eq!(
            cfg.secret_hint("stt.api_key").as_deref(),
            Some("api.groq.com")
        );
    }

    #[test]
    fn pointing_somewhere_else_changes_what_it_says() {
        // Which is the whole reason it is read rather than written down
        // a second time.
        let cfg = crate::Config::load_from_str(
            "[llm]
base_url = \"https://api.openai.com/v1\"
",
        )
        .expect("valid");
        assert_eq!(
            cfg.secret_hint("llm.api_key").as_deref(),
            Some("api.openai.com")
        );
    }

    #[test]
    fn niles_own_secrets_are_used_against_nothing() {
        let cfg = crate::Config::load_from_str("").expect("valid");
        assert_eq!(cfg.secret_hint("auth.session_secret"), None);
        assert_eq!(cfg.secret_hint("auth.api_token"), None);
    }

    #[test]
    fn loading_replaces_rather_than_merges() {
        // A secret somebody deleted has to disappear. Merging would
        // leave it working until the next restart, which is the worst
        // possible time to find out.
        load(HashMap::from([("test.only.gone".into(), "old".into())]));
        load(HashMap::from([("test.only.kept".into(), "new".into())]));
        assert_eq!(get("test.only.gone"), None);
        assert_eq!(get("test.only.kept").as_deref(), Some("new"));
    }
}
