//! Speaker identification wiring for the voice dispatch path.

use std::sync::Arc;

use anyhow::Context;
use niles_config::RecognitionConfig;
use niles_recognition::{
    EcapaTdnnEmbedder, EmbedderConfig, EnrollmentStore, MatchOutcome, Matcher,
};

/// Per-utterance speaker identification. The dispatch path depends on
/// this trait so it can be unit-tested with a mock — the real impl
/// requires the ONNX model on disk.
pub(crate) trait SpeakerIdentifier: Send + Sync {
    /// `Some((display_name, confidence))` on a confident match (and
    /// records last-seen as a side effect); `None` when unknown, the
    /// audio is unusable, or recognition is unavailable.
    fn identify(&self, pcm: &[i16], sample_rate_hz: u32) -> Option<(String, f32)>;
}

/// Map a matcher outcome into an identity, bumping `last_seen_at` on match.
fn outcome_to_identity(outcome: MatchOutcome, store: &EnrollmentStore) -> Option<(String, f32)> {
    match outcome {
        MatchOutcome::Match {
            speaker,
            display_name,
            confidence,
        } => {
            if let Err(e) = store.bump_last_seen(&speaker) {
                tracing::warn!("failed to bump last_seen for {speaker}: {e}");
            }
            Some((display_name, confidence))
        }
        MatchOutcome::Unknown { .. } => None,
        _ => None,
    }
}

/// Convert raw little-endian PCM bytes into `i16` samples.
/// Trailing odd bytes are silently dropped.
pub(crate) fn pcm_bytes_to_i16(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

pub(crate) struct EcapaIdentifier {
    embedder: EcapaTdnnEmbedder,
    matcher: Matcher,
    store: Arc<EnrollmentStore>,
}

impl SpeakerIdentifier for EcapaIdentifier {
    fn identify(&self, pcm: &[i16], sample_rate_hz: u32) -> Option<(String, f32)> {
        let embedding = match self.embedder.extract(pcm, sample_rate_hz) {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!("speaker embedding skipped: {e}");
                return None;
            }
        };
        outcome_to_identity(self.matcher.classify(&embedding), &self.store)
    }
}

/// Build a speaker identifier from config. Returns `None` when disabled.
pub(crate) fn build_speaker_identifier(
    cfg: &RecognitionConfig,
) -> anyhow::Result<Option<Arc<dyn SpeakerIdentifier>>> {
    if !cfg.enabled {
        return Ok(None);
    }

    let model_path = cfg
        .model_path
        .clone()
        .expect("model_path guaranteed by config validation");
    let enrollment_dir = cfg
        .matcher
        .enrollment_dir
        .clone()
        .expect("enrollment_dir guaranteed by config validation");

    let store = EnrollmentStore::open(&enrollment_dir)
        .with_context(|| format!("opening enrollment store at {}", enrollment_dir.display()))?;
    let speakers = store
        .load_all()
        .with_context(|| "loading enrolled speakers")?;
    let matcher = Matcher::new(speakers, cfg.matcher.threshold, cfg.matcher.strategy);
    let embedder = EcapaTdnnEmbedder::new(&EmbedderConfig {
        model_path,
        use_gpu: cfg.use_gpu,
    })
    .context("loading ECAPA-TDNN embedder")?;

    Ok(Some(Arc::new(EcapaIdentifier {
        embedder,
        matcher,
        store: Arc::new(store),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_recognition::MatchStrategy;
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

    #[test]
    fn outcome_to_identity_bumps_last_seen_on_match() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let mut embedding = vec![0.0_f32; 192];
        embedding[0] = 1.0;
        store.enroll("mark", &embedding).unwrap();

        let speakers = store.load_all().unwrap();
        let matcher = Matcher::new(speakers, 0.5, MatchStrategy::MaxSimilarity);

        let outcome = matcher.classify(&embedding);
        let result = outcome_to_identity(outcome, &store);
        let (name, confidence) = result.expect("expected a match");
        assert_eq!(name, "Mark");
        assert!(
            confidence > 0.99,
            "expected high confidence, got {confidence}"
        );

        let record = store.load("mark").unwrap();
        assert!(
            record.last_seen_at.is_some(),
            "last_seen_at should be bumped on match"
        );
    }

    #[test]
    fn outcome_to_identity_no_bump_on_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let mut enrolled = vec![0.0_f32; 192];
        enrolled[0] = 1.0;
        store.enroll("mark", &enrolled).unwrap();

        let speakers = store.load_all().unwrap();
        let matcher = Matcher::new(speakers, 0.5, MatchStrategy::MaxSimilarity);

        let mut query = vec![0.0_f32; 192];
        query[1] = 1.0;
        let outcome = matcher.classify(&query);
        assert!(
            outcome_to_identity(outcome, &store).is_none(),
            "expected no match"
        );

        let record = store.load("mark").unwrap();
        assert!(
            record.last_seen_at.is_none(),
            "last_seen_at should stay None on unknown"
        );
    }

    #[test]
    fn build_speaker_identifier_disabled_returns_none() {
        let cfg = RecognitionConfig::default();
        assert!(build_speaker_identifier(&cfg).unwrap().is_none());
    }
}
