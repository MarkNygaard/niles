//! Speech in and out: the speech-to-text clients, Piper, and speaker enrollment.

use crate::vocabulary;
use anyhow::Context;
use niles_config::{Config, ConfigStore};
use niles_stt::{ScribeClient, ScribeConfig, SttClient, WhisperClient, WhisperConfig};
use niles_tts::{PiperClient, PiperConfig};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Everything a speech-to-text client is built from, resolved.
///
/// Compared whole to decide whether the client in hand is still the one
/// the config asks for — including the key, which is changed in the app
/// like everything else.
#[derive(Clone, PartialEq)]
pub(crate) struct SttSettings {
    pub(crate) api: niles_config::catalogue::Api,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) language: Option<String>,
    pub(crate) keyterms: Vec<String>,
    pub(crate) timeout: Duration,
}

pub(crate) fn stt_settings(cfg: &Config) -> anyhow::Result<SttSettings> {
    // Through the provider when the role names one, and the section's
    // own fields when it does not — so a config written before
    // `[[providers]]` existed behaves exactly as it did.
    let endpoint = cfg
        .endpoint_for(
            niles_config::Role::Stt,
            cfg.stt.provider.as_deref(),
            cfg.stt.base_url.as_deref(),
            &cfg.stt.api_key_env,
            "stt",
        )
        .context("resolving where speech-to-text should go")?;
    // Which client depends on the API the provider speaks, as the
    // catalogue records it. A role on an endpoint of its own speaks the
    // OpenAI shape: a Whisper server, as every provider was until
    // ElevenLabs.
    let api = cfg
        .stt
        .provider_in_use()
        .and_then(niles_config::catalogue::find)
        .map_or(niles_config::catalogue::Api::OpenAi, |known| known.api);
    Ok(SttSettings {
        api,
        base_url: endpoint.base_url,
        api_key: endpoint.api_key,
        model: cfg.stt.model.clone(),
        language: cfg.stt.language.clone(),
        keyterms: cfg.stt.keyterms.clone(),
        timeout: Duration::from_secs(cfg.stt.timeout_seconds),
    })
}

pub(crate) fn build_stt(settings: SttSettings) -> anyhow::Result<SttClient> {
    Ok(match settings.api {
        niles_config::catalogue::Api::ElevenLabs => SttClient::Scribe(
            ScribeClient::new(ScribeConfig {
                api_key: settings.api_key,
                base_url: settings.base_url,
                model: settings.model,
                language: settings.language,
                keyterms: settings.keyterms,
                request_timeout: settings.timeout,
            })
            .context("building Scribe HTTP client")?,
        ),
        niles_config::catalogue::Api::OpenAi => SttClient::Whisper(
            WhisperClient::new(WhisperConfig {
                api_key: settings.api_key,
                base_url: settings.base_url,
                model: settings.model,
                language: settings.language,
                request_timeout: settings.timeout,
            })
            .context("building Whisper HTTP client")?,
        ),
    })
}

/// Build the speech-to-text client the `[stt]` section points at, once.
pub(crate) fn build_stt_client(cfg: &Config) -> anyhow::Result<SttClient> {
    build_stt(stt_settings(cfg)?)
}

/// The speech-to-text client the config asks for *now*.
///
/// It was built once at startup, so choosing ElevenLabs in the app was
/// saved, shown on a page that says changes apply immediately, and
/// ignored for a week — Whisper went on transcribing until something
/// restarted the pod. Now the settings are read for every utterance and
/// the client rebuilt when they differ from the ones it was built from.
///
/// A switch that cannot be built — a provider with no key yet — keeps
/// the client that works and says so, rather than leaving the house
/// unable to hear.
pub(crate) struct LiveStt {
    pub(crate) store: Option<Arc<ConfigStore>>,
    pub(crate) built: Mutex<(SttSettings, Arc<SttClient>)>,
    /// The house's own words, added to the configured keyterms; a new
    /// list is a changed setting, and rebuilds the client like one.
    pub(crate) vocabulary: Arc<vocabulary::Vocabulary>,
}

impl LiveStt {
    pub(crate) fn new(store: Option<Arc<ConfigStore>>, cfg: &Config) -> anyhow::Result<Self> {
        let settings = stt_settings(cfg)?;
        let client = Arc::new(build_stt(settings.clone())?);
        Ok(Self {
            store,
            built: Mutex::new((settings, client)),
            vocabulary: Arc::default(),
        })
    }

    pub(crate) fn vocabulary(&self) -> Arc<vocabulary::Vocabulary> {
        self.vocabulary.clone()
    }

    pub(crate) fn settings(&self, cfg: &Config) -> anyhow::Result<SttSettings> {
        let mut settings = stt_settings(cfg)?;
        settings.keyterms = self.vocabulary.with(&settings.keyterms);
        Ok(settings)
    }

    pub(crate) fn current(&self) -> Arc<SttClient> {
        let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        let Some(store) = self.store.as_ref() else {
            return built.1.clone();
        };
        match self.settings(&store.current()) {
            Ok(settings) if settings == built.0 => {}
            Ok(settings) => match build_stt(settings.clone()) {
                Ok(client) => {
                    tracing::info!(
                        "speech-to-text is now {:?} / {} at {}",
                        settings.api,
                        settings.model,
                        settings.base_url
                    );
                    *built = (settings, Arc::new(client));
                }
                Err(e) => {
                    tracing::warn!("speech-to-text change not applied, keeping the last one: {e:#}")
                }
            },
            Err(e) => {
                tracing::warn!("speech-to-text settings unreadable, keeping the last ones: {e:#}")
            }
        }
        built.1.clone()
    }
}

/// Where speech-to-text requests go, for the line printed at startup.
pub(crate) fn stt_base_url(cfg: &Config) -> String {
    cfg.base_url_for(
        niles_config::Role::Stt,
        cfg.stt.provider.as_deref(),
        cfg.stt.base_url.as_deref(),
    )
    .unwrap_or_default()
}

fn piper_config(cfg: &Config) -> PiperConfig {
    PiperConfig {
        base_url: cfg.tts.base_url.clone(),
        default_voice: cfg.tts.default_voice.clone(),
        request_timeout: Duration::from_secs(cfg.tts.timeout_seconds),
    }
}

/// Build a `PiperClient` from the `[tts]` section of an already-
/// validated config.
pub(crate) fn build_piper_client(cfg: &Config) -> anyhow::Result<PiperClient> {
    PiperClient::new(piper_config(cfg)).context("building Piper HTTP client")
}

/// The Piper client the config asks for *now*, as [`LiveStt`] is for
/// speech-to-text.
///
/// Built once at startup, a new Piper address saved in the app was shown
/// as the effective config while every reply still went to the old one.
/// Now `[tts]` is read for every synthesis and the client rebuilt when it
/// differs. Without a store (a CLI subcommand) the startup config stands.
pub(crate) struct LivePiper {
    store: Option<Arc<ConfigStore>>,
    built: Mutex<Arc<PiperClient>>,
}

impl LivePiper {
    pub(crate) fn new(store: Option<Arc<ConfigStore>>, cfg: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            store,
            built: Mutex::new(Arc::new(build_piper_client(cfg)?)),
        })
    }

    pub(crate) fn current(&self) -> Arc<PiperClient> {
        let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        let Some(store) = self.store.as_ref() else {
            return built.clone();
        };
        let wanted = piper_config(&store.current());
        if &wanted != built.config() {
            match PiperClient::new(wanted.clone()) {
                Ok(client) => {
                    tracing::info!(
                        "text-to-speech is now {} at {}",
                        wanted.default_voice,
                        wanted.base_url
                    );
                    *built = Arc::new(client);
                }
                Err(e) => {
                    tracing::warn!("text-to-speech change not applied, keeping the last one: {e:#}")
                }
            }
        }
        built.clone()
    }
}

pub(crate) fn build_enrollment_backend(
    cfg: &Config,
) -> anyhow::Result<Option<Arc<dyn niles_recognition::EnrollmentBackend>>> {
    if let Some(database) = &cfg.database {
        let url = database
            .resolve_url()
            .context("resolving the database connection string")?;
        let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
            .map_err(|_| {
                anyhow::anyhow!(
                    "the connection string in {} is not a valid database URL",
                    database.url_env
                )
            })?;
        let describe = backend.describe_target();
        return Ok(Some(Arc::new(niles_db::PostgresEnrollments::new(
            backend.pool(),
            describe,
        ))));
    }
    match cfg.recognition.matcher.enrollment_dir.as_ref() {
        Some(dir) => Ok(Some(Arc::new(
            niles_recognition::EnrollmentStore::open(dir)
                .with_context(|| format!("opening enrollment store at {}", dir.display()))?,
        ))),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_config::ChangeSource;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn piper_saying(word: &'static str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/synthesize"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(word.as_bytes()))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn a_changed_piper_address_reaches_the_next_synthesis() {
        let old = piper_saying("old").await;
        let new = piper_saying("new").await;
        let store = Arc::new(
            ConfigStore::from_str_in_memory(&format!("[tts]\nbase_url = \"{}\"\n", old.uri()))
                .unwrap(),
        );
        let piper = LivePiper::new(Some(store.clone()), &store.current()).unwrap();
        let said = piper.current().synthesize("hello", None).await.unwrap();
        assert_eq!(said.audio_wav, b"old");

        let patch: toml::Table =
            toml::from_str(&format!("[tts]\nbase_url = \"{}/synthesize\"\n", new.uri())).unwrap();
        store.apply(&patch, ChangeSource::Api).await.unwrap();

        let said = piper.current().synthesize("hello", None).await.unwrap();
        assert_eq!(said.audio_wav, b"new");
    }

    #[tokio::test]
    async fn an_unchanged_config_keeps_the_same_client() {
        let store = Arc::new(ConfigStore::from_str_in_memory("").unwrap());
        let piper = LivePiper::new(Some(store.clone()), &store.current()).unwrap();
        assert!(Arc::ptr_eq(&piper.current(), &piper.current()));
    }

    #[tokio::test]
    async fn without_a_store_the_startup_config_stands() {
        let old = piper_saying("old").await;
        let store = Arc::new(
            ConfigStore::from_str_in_memory(&format!("[tts]\nbase_url = \"{}\"\n", old.uri()))
                .unwrap(),
        );
        let piper = LivePiper::new(None, &store.current()).unwrap();
        let patch: toml::Table =
            toml::from_str("[tts]\nbase_url = \"http://127.0.0.1:1\"\n").unwrap();
        store.apply(&patch, ChangeSource::Api).await.unwrap();

        let said = piper.current().synthesize("hello", None).await.unwrap();
        assert_eq!(said.audio_wav, b"old");
    }
}
