//! Speaker identification wiring for the voice dispatch path.

use std::sync::Arc;

use anyhow::Context;
use niles_config::RecognitionConfig;
use niles_recognition::{
    EcapaTdnnEmbedder, EmbedderConfig, EnrollmentBackend, MatchOutcome, Matcher,
};
use tokio::sync::mpsc::UnboundedSender;

/// Per-utterance speaker identification. The dispatch path depends on
/// this trait so it can be unit-tested with a mock — the real impl
/// requires the ONNX model on disk.
#[async_trait::async_trait]
pub(crate) trait SpeakerIdentifier: Send + Sync {
    /// The voice print of this utterance. Separate from [`Self::classify`]
    /// because the same print is used twice: to recognise who spoke, and
    /// — when they said "I am Mark" — to enrol them.
    fn embed(&self, pcm: &[i16], sample_rate_hz: u32) -> Option<Vec<f32>>;

    /// `Some((display_name, confidence))` on a confident match, and
    /// records last-seen as a side effect. `None` when unknown.
    fn classify(&self, embedding: &[f32]) -> Option<(String, f32)>;

    /// Teach this voice as `name`, returning how many clips it now has.
    ///
    /// Takes effect immediately — the next sentence is matched against
    /// it — because "I am Mark" followed by not being recognised would
    /// read as the feature simply not working.
    async fn enroll(&self, name: &str, embedding: &[f32]) -> anyhow::Result<usize>;

    /// Whether `name` is already enrolled and this voice is not it.
    /// Adding clips to someone else's identity is how you become them.
    async fn is_someone_else(&self, name: &str, embedding: &[f32]) -> bool;

    /// The *slug* of whoever this voice belongs to, if anybody.
    ///
    /// Separate from [`Self::classify`], which answers with the display
    /// name because it is feeding a spoken reply. Enrolment needs the
    /// key the store is indexed by, and "Mark" is not "mark".
    fn whose_voice(&self, embedding: &[f32]) -> Option<String>;

    /// Whether anybody is enrolled at all.
    ///
    /// The lock checks this before it refuses anyone: a house where
    /// nobody has introduced themselves has no known voices, so
    /// "only answer voices I know" would answer nobody — including
    /// whoever wants to turn it back off.
    fn knows_anybody(&self) -> bool;
}

#[async_trait::async_trait]
impl niles_recognition::VoiceRoster for EcapaIdentifier {
    async fn voices(&self) -> niles_recognition::Result<Vec<niles_recognition::EnrolledSpeaker>> {
        self.backend.load_all().await
    }

    async fn rename(&self, speaker: &str, display_name: &str) -> niles_recognition::Result<()> {
        self.backend.set_display_name(speaker, display_name).await?;
        // The matcher carries display names into spoken replies, so a
        // rename that only reached the store would have Niles going on
        // greeting somebody by the name they just corrected.
        let speakers = self.backend.load_all().await?;
        let next = Matcher::new(speakers, self.threshold, self.strategy);
        *self.matcher.write().unwrap_or_else(|e| e.into_inner()) = next;
        Ok(())
    }

    async fn forget(&self, speaker: &str) -> niles_recognition::Result<()> {
        self.backend.delete(speaker).await?;
        // The matcher holds its roster in memory, so a delete that
        // only reached the store would leave Niles recognising a voice
        // it had been told to forget until something restarted it.
        let speakers = self.backend.load_all().await?;
        let next = Matcher::new(speakers, self.threshold, self.strategy);
        *self.matcher.write().unwrap_or_else(|e| e.into_inner()) = next;
        Ok(())
    }
}

/// Map a matcher outcome into an identity, reporting the sighting.
///
/// `heard` is a send, not a write: identification runs on a blocking
/// thread and the store may be a database. A dropped timestamp costs
/// nothing, and making the caller wait for a round trip to record one
/// would cost the thing this path exists to protect.
fn outcome_to_identity(
    outcome: MatchOutcome,
    heard: &UnboundedSender<String>,
) -> Option<(String, f32)> {
    match outcome {
        MatchOutcome::Match {
            speaker,
            display_name,
            confidence,
        } => {
            let _ = heard.send(speaker);
            Some((display_name, confidence))
        }
        // Logged rather than dropped. "Not recognised" on its own gives
        // nobody anything to tune: the threshold is a number, and the
        // only way to know whether it is the wrong one is to see what
        // the voice actually scored against it.
        MatchOutcome::Unknown {
            best_similarity,
            ref nearest_speaker,
        } => {
            tracing::info!(
                best_similarity,
                nearest = nearest_speaker.as_deref().unwrap_or("nobody"),
                "speaker not recognised"
            );
            None
        }
        _ => None,
    }
}

/// Convert raw little-endian PCM bytes into `i16` samples.
/// Trailing odd bytes are silently dropped.
pub(crate) fn pcm_bytes_to_i16(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect()
}

pub(crate) struct EcapaIdentifier {
    embedder: EcapaTdnnEmbedder,
    /// Behind a lock because enrolling by voice replaces it while the
    /// process runs. Read on every turn, written once in a blue moon.
    matcher: std::sync::RwLock<Matcher>,
    backend: Arc<dyn EnrollmentBackend>,
    threshold: f32,
    strategy: niles_recognition::MatchStrategy,
    heard: UnboundedSender<String>,
}

impl EcapaIdentifier {
    fn read_matcher(&self) -> std::sync::RwLockReadGuard<'_, Matcher> {
        self.matcher
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait::async_trait]
impl SpeakerIdentifier for EcapaIdentifier {
    fn embed(&self, pcm: &[i16], sample_rate_hz: u32) -> Option<Vec<f32>> {
        match self.embedder.extract(pcm, sample_rate_hz) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::debug!("speaker embedding skipped: {e}");
                None
            }
        }
    }

    fn classify(&self, embedding: &[f32]) -> Option<(String, f32)> {
        outcome_to_identity(self.read_matcher().classify(embedding), &self.heard)
    }

    fn whose_voice(&self, embedding: &[f32]) -> Option<String> {
        match self.read_matcher().classify(embedding) {
            MatchOutcome::Match { speaker, .. } => Some(speaker),
            _ => None,
        }
    }

    fn knows_anybody(&self) -> bool {
        self.read_matcher().knows_anybody()
    }

    async fn enroll(&self, name: &str, embedding: &[f32]) -> anyhow::Result<usize> {
        self.backend
            .enroll(name, embedding)
            .await
            .with_context(|| format!("enrolling {name}"))?;
        let speakers = self
            .backend
            .load_all()
            .await
            .context("reloading enrolled speakers")?;
        let clips = speakers
            .iter()
            .find(|s| s.speaker == name)
            .map_or(0, |s| s.clip_count);
        let next = Matcher::new(speakers, self.threshold, self.strategy);
        *self
            .matcher
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = next;
        Ok(clips)
    }

    async fn is_someone_else(&self, name: &str, embedding: &[f32]) -> bool {
        let enrolled = match self.backend.load(name).await {
            Ok(record) => record,
            // Nobody by that name yet, so nobody to impersonate.
            Err(_) => return false,
        };
        if enrolled.embeddings.is_empty() {
            return false;
        }
        // Refuse only a voice that is confidently *somebody else*.
        //
        // This used to refuse anything that did not already classify as
        // the claimed name, which made enrolment unable to finish: one
        // clip is too thin to match against, so the second "my name is
        // Mark" was turned away by the first. Niles asked for one or
        // two more and then refused them — the reply and the guard
        // disagreed, and the guard won.
        //
        // Unknown is the ordinary case while somebody is still being
        // learned, and it has to be allowed for the clips to accumulate
        // at all. What that opens — a stranger adding themselves to a
        // name nobody is watching — is what `known_voices_only` is for,
        // and enrolment is meant to be done with that switch off.
        self.read_matcher()
            .classify(embedding)
            .is_someone_other_than(&enrolled.speaker)
    }
}

/// Build a speaker identifier from config. Returns `None` when disabled.
///
/// Enrolled voices are read once, here: matching is a cosine similarity
/// against a handful of vectors, so the hot path holds them in memory
/// and never touches storage. A voice enrolled afterwards is picked up
/// on the next restart.
pub(crate) async fn build_speaker_identifier(
    cfg: &RecognitionConfig,
    backend: Option<Arc<dyn EnrollmentBackend>>,
) -> anyhow::Result<Option<Arc<EcapaIdentifier>>> {
    if !cfg.enabled {
        return Ok(None);
    }

    let model_path = cfg
        .model_path
        .clone()
        .expect("model_path guaranteed by config validation");
    let backend = backend.context(
        "recognition is enabled but there is nowhere to keep enrolled voices:          configure [database], or [recognition.matcher] enrollment_dir",
    )?;

    let speakers = backend
        .load_all()
        .await
        .with_context(|| format!("loading enrolled speakers from {}", backend.describe()))?;
    if speakers.is_empty() {
        tracing::warn!(
            "speaker recognition is on but nobody is enrolled in {};              every turn will be 'not recognized' until `niles enroll` is run",
            backend.describe()
        );
    } else {
        tracing::info!(
            "speaker recognition: {} enrolled from {}",
            speakers.len(),
            backend.describe()
        );
    }
    let matcher = Matcher::new(speakers, cfg.matcher.threshold, cfg.matcher.strategy);
    let embedder = EcapaTdnnEmbedder::new(&EmbedderConfig {
        model_path,
        use_gpu: cfg.use_gpu,
    })
    .context("loading ECAPA-TDNN embedder")?;

    let heard = spawn_last_seen_writer(backend.clone());
    Ok(Some(Arc::new(EcapaIdentifier {
        embedder,
        matcher: std::sync::RwLock::new(matcher),
        backend,
        threshold: cfg.matcher.threshold,
        strategy: cfg.matcher.strategy,
        heard,
    })))
}

/// Drain recognised speakers into the store, off the dispatch path.
fn spawn_last_seen_writer(backend: Arc<dyn EnrollmentBackend>) -> UnboundedSender<String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(speaker) = rx.recv().await {
            if let Err(e) = backend.bump_last_seen(&speaker).await {
                tracing::warn!("failed to record that {speaker} was heard: {e}");
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_recognition::{EnrollmentStore, MatchStrategy};
    #[test]
    fn pcm_bytes_to_i16_round_trip() {
        let bytes: Vec<u8> = vec![0x01, 0x00, 0x00, 0x02];
        let samples = pcm_bytes_to_i16(&bytes);
        assert_eq!(samples, vec![1, 512]);
    }

    #[test]
    fn pcm_bytes_to_i16_drops_odd_byte() {
        let bytes: Vec<u8> = vec![0x01, 0x00, 0xAB];
        let samples = pcm_bytes_to_i16(&bytes);
        assert_eq!(samples, vec![1]);
    }

    #[tokio::test]
    async fn outcome_to_identity_reports_a_match_without_waiting_on_storage() {
        // Identification runs on a blocking thread and the store may be
        // a database; the sighting is sent, not written.
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let mut embedding = vec![0.0_f32; 192];
        embedding[0] = 1.0;
        EnrollmentBackend::enroll(&store, "mark", &embedding)
            .await
            .unwrap();

        let speakers = EnrollmentBackend::load_all(&store).await.unwrap();
        let matcher = Matcher::new(speakers, 0.5, MatchStrategy::MaxSimilarity);
        let heard = spawn_last_seen_writer(Arc::new(store));

        let (name, confidence) =
            outcome_to_identity(matcher.classify(&embedding), &heard).expect("expected a match");
        assert_eq!(name, "Mark");
        assert!(
            confidence > 0.99,
            "expected high confidence, got {confidence}"
        );
    }

    #[tokio::test]
    async fn an_unknown_voice_is_not_reported_as_heard() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let mut enrolled = vec![0.0_f32; 192];
        enrolled[0] = 1.0;
        EnrollmentBackend::enroll(&store, "mark", &enrolled)
            .await
            .unwrap();

        let speakers = EnrollmentBackend::load_all(&store).await.unwrap();
        let matcher = Matcher::new(speakers, 0.5, MatchStrategy::MaxSimilarity);
        let (heard, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let mut query = vec![0.0_f32; 192];
        query[1] = 1.0;
        assert!(
            outcome_to_identity(matcher.classify(&query), &heard).is_none(),
            "expected no match"
        );
        assert!(
            rx.try_recv().is_err(),
            "nobody was heard, so nothing to record"
        );
    }

    #[tokio::test]
    async fn a_recognised_speaker_is_recorded_as_heard() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(EnrollmentStore::open(tmp.path()).unwrap());
        let mut embedding = vec![0.0_f32; 192];
        embedding[0] = 1.0;
        EnrollmentBackend::enroll(store.as_ref(), "mark", &embedding)
            .await
            .unwrap();

        let speakers = EnrollmentBackend::load_all(store.as_ref()).await.unwrap();
        let matcher = Matcher::new(speakers, 0.5, MatchStrategy::MaxSimilarity);
        let heard = spawn_last_seen_writer(store.clone());
        outcome_to_identity(matcher.classify(&embedding), &heard).expect("expected a match");

        // The write lands on a background task, so wait for it rather
        // than assuming it already happened.
        for _ in 0..50 {
            let record = EnrollmentBackend::load(store.as_ref(), "mark")
                .await
                .unwrap();
            if record.last_seen_at.is_some() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("last_seen_at was never recorded");
    }

    #[tokio::test]
    async fn build_speaker_identifier_disabled_returns_none() {
        let cfg = RecognitionConfig::default();
        assert!(
            build_speaker_identifier(&cfg, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn recognition_without_anywhere_to_keep_voices_is_an_error() {
        // Silently starting with nobody enrolled would look identical
        // to nobody ever being recognised.
        let cfg = RecognitionConfig {
            enabled: true,
            model_path: Some(std::path::PathBuf::from("/models/ecapa.onnx")),
            ..Default::default()
        };
        let err = match build_speaker_identifier(&cfg, None).await {
            Err(e) => e,
            Ok(_) => panic!("expected an error"),
        };
        assert!(err.to_string().contains("nowhere to keep"), "{err}");
    }
}
