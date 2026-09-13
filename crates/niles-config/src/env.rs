//! Reading credentials from the environment.
//!
//! Config files name the variable; the value never appears in them, so
//! this is the one place a secret is fetched.
//!
//! An empty variable counts as unset. A secret rendered as `""` is a
//! placeholder nobody filled in, and letting it through means the process
//! starts happily and then fails at the first API call — which is how a
//! missing Groq key presented as "the voice satellite gets no reply",
//! three layers away from the cause.

use crate::error::{Error, Result};

/// Read `var`, treating empty or whitespace-only as not set.
///
/// `section` names the config section for the error message, so the
/// failure says which setting to go and look at.
/// A secret, from wherever it actually lives.
///
/// Two places, in this order: the environment variable the config
/// names, then whatever was saved under `key`. Environment first so an
/// existing install is unaffected by any of this — a variable that is
/// set still wins, and nothing has to be migrated.
///
/// `key` says what the secret is *for* (`mqtt.password`), not where it
/// was kept, so it survives somebody renaming the variable.
pub(crate) fn require_secret(section: &'static str, key: &str, var: &str) -> Result<String> {
    if !var.trim().is_empty()
        && let Ok(value) = std::env::var(var)
        && !value.trim().is_empty()
    {
        return Ok(value);
    }
    if let Some(value) = crate::secrets::get(key) {
        return Ok(value);
    }
    // Named a variable that is not set: that is the more specific
    // complaint, and the more likely mistake.
    if !var.trim().is_empty() {
        return require_env(section, var);
    }
    Err(Error::InvalidSection {
        section,
        reason: format!("no {key} has been set — add one in Settings"),
    })
}

pub(crate) fn require_env(section: &'static str, var: &str) -> Result<String> {
    let value = std::env::var(var).map_err(|_| Error::InvalidSection {
        section,
        reason: format!("env var {var} is not set"),
    })?;
    if value.trim().is_empty() {
        return Err(Error::InvalidSection {
            section,
            // Distinct wording on purpose: "set but empty" points at the
            // secret's contents, "not set" at the secret's absence. They
            // are different fixes.
            reason: format!("env var {var} is set but empty"),
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_variable_is_returned() {
        unsafe { std::env::set_var("NILES_TEST_ENV_OK", "secret-value") };
        assert_eq!(
            require_env("stt", "NILES_TEST_ENV_OK").unwrap(),
            "secret-value"
        );
    }

    #[test]
    fn a_missing_variable_says_it_is_not_set() {
        let err = require_env("stt", "NILES_TEST_ENV_ABSENT").unwrap_err();
        assert!(err.to_string().contains("is not set"), "{err}");
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        // The failure this exists for: an empty secret placeholder that
        // starts the process and breaks the first API call instead.
        unsafe { std::env::set_var("NILES_TEST_ENV_EMPTY", "") };
        let err = require_env("stt", "NILES_TEST_ENV_EMPTY").unwrap_err();
        assert!(err.to_string().contains("set but empty"), "{err}");
    }

    #[test]
    fn whitespace_only_counts_as_unset() {
        unsafe { std::env::set_var("NILES_TEST_ENV_BLANK", "   \n") };
        assert!(require_env("stt", "NILES_TEST_ENV_BLANK").is_err());
    }

    #[test]
    fn the_error_names_the_variable_and_the_section() {
        let err = require_env("llm", "NILES_TEST_ENV_NAMED").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("NILES_TEST_ENV_NAMED"), "{text}");
        assert!(text.contains("llm"), "{text}");
    }
}
