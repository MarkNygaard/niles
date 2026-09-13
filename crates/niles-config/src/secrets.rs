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
