//! Speaker matcher — classify a query embedding against an enrolled roster.

use crate::enrollment::{EnrolledSpeaker, EnrollmentEntry};
use serde::{Deserialize, Serialize};

/// Strategy for aggregating a speaker's multiple embeddings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MatchStrategy {
    /// Use the highest cosine similarity across all entries.
    #[default]
    MaxSimilarity,
    /// Use the centroid (mean) of all entries, L2-normalized.
    Centroid,
}

/// Result of matching a query against the roster.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MatchOutcome {
    Match {
        speaker: String,
        display_name: String,
        confidence: f32,
    },
    Unknown {
        best_similarity: f32,
        nearest_speaker: Option<String>,
    },
}

/// Classifies query embeddings against enrolled speakers.
pub struct Matcher {
    speakers: Vec<EnrolledSpeaker>,
    threshold: f32,
    strategy: MatchStrategy,
    centroids: Option<Vec<Option<Vec<f32>>>>,
}

impl Matcher {
    /// Build a matcher from a roster snapshot.
    pub fn new(speakers: Vec<EnrolledSpeaker>, threshold: f32, strategy: MatchStrategy) -> Self {
        let centroids = if strategy == MatchStrategy::Centroid {
            Some(speakers.iter().map(|s| centroid(&s.embeddings)).collect())
        } else {
            None
        };
        Self {
            speakers,
            threshold,
            strategy,
            centroids,
        }
    }

    /// Classify `query` against the roster.
    ///
    /// # Panics
    ///
    /// Panics if `query` is not 192-dim.
    pub fn classify(&self, query: &[f32]) -> MatchOutcome {
        assert_eq!(query.len(), 192, "query embedding must be 192-dim");

        if self.speakers.is_empty() {
            return MatchOutcome::Unknown {
                best_similarity: -1.0,
                nearest_speaker: None,
            };
        }

        let mut best_score = f32::NEG_INFINITY;
        let mut best: Option<(&str, &str)> = None;

        for (i, speaker) in self.speakers.iter().enumerate() {
            let score = match self.strategy {
                MatchStrategy::MaxSimilarity => {
                    if speaker.embeddings.is_empty() {
                        continue;
                    }
                    speaker
                        .embeddings
                        .iter()
                        .map(|e| crate::similarity::cosine_similarity(query, &e.embedding))
                        .fold(f32::NEG_INFINITY, f32::max)
                }
                MatchStrategy::Centroid => {
                    let centroids = self.centroids.as_ref().unwrap();
                    match &centroids[i] {
                        Some(c) => crate::similarity::cosine_similarity(query, c),
                        None => continue,
                    }
                }
            };

            if score > best_score {
                best_score = score;
                best = Some((&speaker.speaker, &speaker.display_name));
            }
        }

        if best_score >= self.threshold {
            let (speaker, display_name) = best
                .map(|(s, d)| (s.to_string(), d.to_string()))
                .unwrap_or_default();
            MatchOutcome::Match {
                speaker,
                display_name,
                confidence: best_score,
            }
        } else {
            MatchOutcome::Unknown {
                best_similarity: best_score,
                nearest_speaker: best.map(|(s, _)| s.to_string()),
            }
        }
    }
}

/// Compute the L2-normalized centroid of a list of embeddings.
/// Returns `None` when the input is empty.
fn centroid(embeddings: &[EnrollmentEntry]) -> Option<Vec<f32>> {
    if embeddings.is_empty() {
        return None;
    }
    let dim = embeddings[0].embedding.len();
    let mut sum = vec![0.0_f32; dim];
    for e in embeddings {
        for (i, v) in e.embedding.iter().enumerate() {
            sum[i] += v;
        }
    }
    let count = embeddings.len() as f32;
    for s in sum.iter_mut() {
        *s /= count;
    }
    crate::similarity::l2_normalize(&mut sum);
    Some(sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrollment::EnrollmentEntry;
    use chrono::Utc;
    use rand::SeedableRng;
    use rand::distr::Distribution;

    fn synth_one(seed: u64) -> Vec<f32> {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let mut v: Vec<f32> = (0..192)
            .map(|_| {
                rand::distr::Uniform::new(-1.0_f32, 1.0)
                    .unwrap()
                    .sample(&mut rng)
            })
            .collect();
        crate::similarity::l2_normalize(&mut v);
        v
    }

    fn synth(seed: u64, n: usize) -> Vec<EnrollmentEntry> {
        let base = synth_one(seed);
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed + 1);
        (0..n)
            .map(|_| {
                let mut e = base.clone();
                for x in e.iter_mut() {
                    *x += rand::distr::Uniform::new(-0.01_f32, 0.01)
                        .unwrap()
                        .sample(&mut rng);
                }
                crate::similarity::l2_normalize(&mut e);
                EnrollmentEntry {
                    recorded_at: Utc::now(),
                    embedding: e,
                }
            })
            .collect()
    }

    fn make_speaker(name: &str, entries: Vec<EnrollmentEntry>) -> EnrolledSpeaker {
        EnrolledSpeaker {
            speaker: name.to_string(),
            display_name: name.to_string(),
            created_at: Utc::now(),
            last_seen_at: None,
            clip_count: entries.len(),
            embeddings: entries,
        }
    }

    #[test]
    fn empty_matcher_returns_unknown() {
        let query = synth_one(1);
        let matcher = Matcher::new(vec![], 0.5, MatchStrategy::MaxSimilarity);
        let outcome = matcher.classify(&query);
        assert_eq!(
            outcome,
            MatchOutcome::Unknown {
                best_similarity: -1.0,
                nearest_speaker: None,
            }
        );
    }

    #[test]
    fn single_speaker_self_match() {
        let e = synth(1, 1);
        let query = e[0].embedding.clone();
        let matcher = Matcher::new(
            vec![make_speaker("a", e)],
            0.5,
            MatchStrategy::MaxSimilarity,
        );
        let outcome = matcher.classify(&query);
        assert!(
            matches!(outcome, MatchOutcome::Match { speaker, confidence, .. } if speaker == "a" && confidence > 0.99)
        );
    }

    #[test]
    fn single_speaker_low_similarity_unknown() {
        let e = synth(1, 1);
        let query = synth_one(99);
        let matcher = Matcher::new(
            vec![make_speaker("a", e)],
            0.5,
            MatchStrategy::MaxSimilarity,
        );
        let outcome = matcher.classify(&query);
        assert!(
            matches!(outcome, MatchOutcome::Unknown { nearest_speaker: Some(ref s), .. } if s == "a")
        );
    }

    #[test]
    fn two_speakers_pick_closer() {
        let e_a = synth(1, 1);
        let query = e_a[0].embedding.clone();
        let e_b = synth(99, 1);
        let matcher = Matcher::new(
            vec![make_speaker("a", e_a), make_speaker("b", e_b)],
            0.5,
            MatchStrategy::MaxSimilarity,
        );
        let outcome = matcher.classify(&query);
        assert!(matches!(outcome, MatchOutcome::Match { speaker, .. } if speaker == "a"));
    }

    #[test]
    fn high_threshold_demotes_to_unknown() {
        let e = synth(1, 1);
        let query = e[0].embedding.clone();
        let matcher = Matcher::new(
            vec![make_speaker("a", e)],
            1.0001,
            MatchStrategy::MaxSimilarity,
        );
        let outcome = matcher.classify(&query);
        assert!(
            matches!(outcome, MatchOutcome::Unknown { nearest_speaker: Some(ref s), .. } if s == "a")
        );
    }

    #[test]
    fn centroid_strategy_outscores_individual() {
        let entries = synth(42, 3);
        let query = synth_one(42);
        let speaker = make_speaker("a", entries.clone());
        let max_matcher = Matcher::new(vec![speaker.clone()], 0.0, MatchStrategy::MaxSimilarity);
        let centroid_matcher = Matcher::new(vec![speaker], 0.0, MatchStrategy::Centroid);

        let max_outcome = max_matcher.classify(&query);
        let centroid_outcome = centroid_matcher.classify(&query);

        let max_conf = match max_outcome {
            MatchOutcome::Match { confidence, .. } => confidence,
            MatchOutcome::Unknown {
                best_similarity, ..
            } => best_similarity,
        };
        let centroid_conf = match centroid_outcome {
            MatchOutcome::Match { confidence, .. } => confidence,
            MatchOutcome::Unknown {
                best_similarity, ..
            } => best_similarity,
        };
        assert!(centroid_conf >= max_conf);
    }

    #[test]
    fn max_similarity_skips_empty_embeddings() {
        let empty_speaker = EnrolledSpeaker {
            speaker: "empty".to_string(),
            display_name: "Empty".to_string(),
            created_at: Utc::now(),
            last_seen_at: None,
            clip_count: 0,
            embeddings: vec![],
        };
        let query = synth_one(1);
        let matcher = Matcher::new(vec![empty_speaker], 0.5, MatchStrategy::MaxSimilarity);
        let outcome = matcher.classify(&query);
        assert!(matches!(
            outcome,
            MatchOutcome::Unknown {
                nearest_speaker: None,
                ..
            }
        ));
    }

    #[test]
    fn centroid_skips_empty_embeddings() {
        let empty_speaker = EnrolledSpeaker {
            speaker: "empty".to_string(),
            display_name: "Empty".to_string(),
            created_at: Utc::now(),
            last_seen_at: None,
            clip_count: 0,
            embeddings: vec![],
        };
        let query = synth_one(1);
        let matcher = Matcher::new(vec![empty_speaker], 0.5, MatchStrategy::Centroid);
        let outcome = matcher.classify(&query);
        assert!(matches!(
            outcome,
            MatchOutcome::Unknown {
                nearest_speaker: None,
                ..
            }
        ));
    }
}
