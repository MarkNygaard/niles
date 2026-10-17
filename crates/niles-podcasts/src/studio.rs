//! The studio: the voices and the bucket together, read from the live
//! config at every use, so the add-on comes and goes without a restart.

use crate::error::{Error, Result};
use crate::{Bucket, Kokoro, Speech, Storage};
use std::sync::Arc;

/// What the add-on still needs, in the order a person fills it in:
/// the two addresses, the bucket, the two halves of the API token.
/// Empty means complete.
pub fn missing(cfg: &niles_config::PodcastsConfig) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if cfg.kokoro_address().is_none() {
        missing.push("Kokoro address");
    }
    if cfg.endpoint_address().is_none() {
        missing.push("R2 endpoint");
    }
    if cfg.bucket_name().is_none() {
        missing.push("bucket name");
    }
    if cfg.access_key_id().is_err() {
        missing.push("access key ID");
    }
    if cfg.secret_access_key().is_err() {
        missing.push("secret access key");
    }
    missing
}

/// Everything recording an episode needs, from the config as it is
/// now. `storage` is the trait's view of the bucket, for the one
/// caller that speaks trait.
#[derive(Clone)]
pub struct Studio {
    pub voices: [String; 2],
    pub keep_days: u16,
    pub speech: Arc<dyn Speech>,
    pub storage: Arc<dyn Storage>,
}

impl Studio {
    pub fn from_config(cfg: &niles_config::PodcastsConfig) -> Result<Studio> {
        let missing = missing(cfg);
        if !missing.is_empty() {
            return Err(Error::NotSetUp { missing });
        }
        Ok(Studio {
            voices: cfg.voices(),
            keep_days: cfg.keep_days,
            speech: Arc::new(Kokoro::from_config(cfg)?),
            storage: Arc::new(Bucket::from_config(cfg)?),
        })
    }
}

/// How a voice sounds, for the prompt: the id's first two letters are
/// accent and sex in Kokoro's naming (`af_`, `am_`, `bf_`, `bm_`).
pub fn voice_hint(voice: &str) -> Option<&'static str> {
    match voice.get(..2) {
        Some("af") => Some("a woman's voice with an American accent"),
        Some("am") => Some("a man's voice with an American accent"),
        Some("bf") => Some("a woman's voice with a British accent"),
        Some("bm") => Some("a man's voice with a British accent"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(body: &str) -> niles_config::PodcastsConfig {
        toml::from_str::<niles_config::IntegrationsConfig>(&format!("[podcasts]\n{body}"))
            .expect("valid")
            .podcasts
            .expect("the section")
    }

    #[test]
    fn missing_lists_exactly_what_is_blank() {
        // SAFETY: test-only variable names, unique to this test; no
        // other test reads them.
        unsafe {
            std::env::set_var("NILES_TEST_PODCASTS_STUDIO_AKID", "akid");
            std::env::set_var("NILES_TEST_PODCASTS_STUDIO_SECRET", "secret");
        }
        let with_keys = |cfg: &mut niles_config::PodcastsConfig| {
            cfg.access_key_id_env = "NILES_TEST_PODCASTS_STUDIO_AKID".into();
            cfg.secret_access_key_env = "NILES_TEST_PODCASTS_STUDIO_SECRET".into();
        };
        let mut cfg = config("");
        with_keys(&mut cfg);
        assert_eq!(
            missing(&cfg),
            ["Kokoro address", "R2 endpoint", "bucket name"]
        );
        cfg.kokoro_url = "http://kokoro:8880".into();
        cfg.endpoint = "https://account.r2.cloudflarestorage.com".into();
        cfg.bucket = "episodes".into();
        assert!(missing(&cfg).is_empty());
        let studio = Studio::from_config(&cfg).expect("a studio");
        assert_eq!(studio.voices, ["af_heart", "am_michael"]);
        assert_eq!(studio.keep_days, 14);
        // Without the keys, exactly those two are missing.
        let mut bare = config("");
        assert_eq!(
            missing(&bare),
            [
                "Kokoro address",
                "R2 endpoint",
                "bucket name",
                "access key ID",
                "secret access key"
            ]
        );
        bare.endpoint = "https://account.r2.cloudflarestorage.com".into();
        assert_eq!(
            missing(&bare),
            [
                "Kokoro address",
                "bucket name",
                "access key ID",
                "secret access key"
            ]
        );
    }

    #[test]
    fn voice_hint_reads_accent_and_sex() {
        assert_eq!(
            voice_hint("af_heart"),
            Some("a woman's voice with an American accent")
        );
        assert_eq!(
            voice_hint("am_michael"),
            Some("a man's voice with an American accent")
        );
        assert_eq!(
            voice_hint("bm_george"),
            Some("a man's voice with a British accent")
        );
        assert_eq!(
            voice_hint("bf_emma"),
            Some("a woman's voice with a British accent")
        );
        assert_eq!(voice_hint("ef_dora"), None);
        assert_eq!(voice_hint("zz"), None);
    }
}
