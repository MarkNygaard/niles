//! Topic detection — pick relevant capability names from a transcript.
//!
//! Tokenizes a transcript and returns the subset of capability names
//! whose name/description tokens intersect the transcript tokens,
//! plus the transitive closure of their prerequisites.

use std::collections::HashSet;

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "do", "for", "from", "had", "has", "have",
    "he", "her", "his", "i", "in", "is", "it", "its", "of", "on", "or", "she", "that", "the", "to",
    "was", "were", "what", "when", "where", "who", "will", "with", "you", "your",
];

/// An indexed capability entry.
#[derive(Debug, Clone)]
pub struct CapabilityIndexEntry {
    pub name: String,
    pub description: String,
    pub prerequisites: Vec<String>,
}

/// A collection of capabilities that can be searched by token intersection.
#[derive(Debug, Clone)]
pub struct CapabilityIndex {
    entries: Vec<CapabilityIndexEntry>,
    by_name: std::collections::HashMap<String, usize>,
    token_sets: Vec<HashSet<String>>,
}

impl CapabilityIndex {
    pub fn new() -> Self {
        Self::from_entries(Vec::new())
    }

    pub fn from_entries(entries: Vec<CapabilityIndexEntry>) -> Self {
        let by_name = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.clone(), i))
            .collect();
        let token_sets = entries
            .iter()
            .map(|e| {
                tokenize(&format!("{} {}", e.name, e.description))
                    .into_iter()
                    .collect()
            })
            .collect();
        Self {
            entries,
            by_name,
            token_sets,
        }
    }

    pub fn entries(&self) -> &[CapabilityIndexEntry] {
        &self.entries
    }

    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for CapabilityIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Tokenize a string: lowercase, split on whitespace, strip non-alphanumeric,
/// and drop English stop-words.
pub fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        })
        .filter(|word| !word.is_empty() && !STOP_WORDS.contains(&word.as_str()))
        .collect()
}

/// Detect relevant capability topics from a transcript.
///
/// Returns capability names whose tokens intersect the transcript tokens,
/// including the transitive closure of prerequisites, sorted alphabetically.
pub fn detect_topics(transcript: &str, index: &CapabilityIndex) -> Vec<String> {
    let transcript_tokens: HashSet<String> = tokenize(transcript).into_iter().collect();
    if transcript_tokens.is_empty() {
        return Vec::new();
    }

    let mut matched = HashSet::new();
    for (i, entry) in index.entries.iter().enumerate() {
        if !index.token_sets[i].is_disjoint(&transcript_tokens) {
            matched.insert(entry.name.as_str());
        }
    }

    let mut visited = HashSet::new();
    let mut worklist: Vec<&str> = matched.iter().copied().collect();
    while let Some(name) = worklist.pop() {
        if !visited.insert(name) {
            continue;
        }
        if let Some(&idx) = index.by_name.get(name) {
            for prereq in &index.entries[idx].prerequisites {
                worklist.push(prereq.as_str());
            }
        }
    }

    let mut result: Vec<String> = visited.into_iter().map(|s| s.to_string()).collect();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, description: &str, prerequisites: &[&str]) -> CapabilityIndexEntry {
        CapabilityIndexEntry {
            name: name.to_string(),
            description: description.to_string(),
            prerequisites: prerequisites.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn detect(transcript: &str, entries: Vec<CapabilityIndexEntry>) -> Vec<String> {
        let index = CapabilityIndex::from_entries(entries);
        detect_topics(transcript, &index)
    }

    // 1. Empty index → empty result
    #[test]
    fn empty_index_returns_empty() {
        assert_eq!(
            detect_topics("turn on the lights", &CapabilityIndex::new()),
            Vec::<String>::new()
        );
    }

    // 2. `lighting` capability + "turn on the lights" → `["lighting"]`
    #[test]
    fn lighting_matches_lights() {
        let entries = vec![entry("lighting", "control lights in your home", &[])];
        assert_eq!(detect("turn on the lights", entries), vec!["lighting"]);
    }

    // 3. `scenes` capability + "save a scene" → `["scenes"]`
    #[test]
    fn scenes_matches_scene() {
        let entries = vec![entry("scenes", "save and recall lighting scenes", &[])];
        assert_eq!(detect("save a scene", entries), vec!["scenes"]);
    }

    // 4. Two capabilities, one matches → only that one returned
    #[test]
    fn one_of_two_matches() {
        let entries = vec![
            entry("lighting", "control lights", &[]),
            entry("climate", "control temperature", &[]),
        ];
        assert_eq!(detect("turn on the lights", entries), vec!["lighting"]);
    }

    // 5. Two capabilities, both match → both returned (alphabetical order)
    #[test]
    fn both_match_sorted() {
        let entries = vec![
            entry("lighting", "control lights", &[]),
            entry("scenes", "save and recall lighting scenes", &[]),
        ];
        assert_eq!(
            detect("save the lights", entries),
            vec!["lighting", "scenes"]
        );
    }

    // 6. Capability matches only via description tokens → still returned
    #[test]
    fn description_only_match() {
        let entries = vec![entry("ambiance", "mood lighting for your rooms", &[])];
        assert_eq!(detect("mood lighting", entries), vec!["ambiance"]);
    }

    // 7. Prerequisite resolution: lighting → devices
    #[test]
    fn prerequisite_expansion() {
        let entries = vec![
            entry("lighting", "control lights", &["devices"]),
            entry("devices", "manage your devices", &[]),
        ];
        assert_eq!(
            detect("turn on the lights", entries),
            vec!["devices", "lighting"]
        );
    }

    // 8. Two-level chain: morning → lighting → devices
    #[test]
    fn two_level_prerequisite_chain() {
        let entries = vec![
            entry("morning", "morning routine", &["lighting"]),
            entry("lighting", "control lights", &["devices"]),
            entry("devices", "manage your devices", &[]),
        ];
        assert_eq!(
            detect("morning", entries),
            vec!["devices", "lighting", "morning"]
        );
    }

    // 9. Cycle in prerequisites (a → b → a) → terminates, returns both
    #[test]
    fn cycle_in_prerequisites_terminates() {
        let entries = vec![
            entry("a", "capability a", &["b"]),
            entry("b", "capability b", &["a"]),
        ];
        // "capability" is in both descriptions; triggers cycle-safe expansion.
        assert_eq!(detect("capability", entries), vec!["a", "b"]);
    }

    // 10. Transcript with no non-stopword tokens → empty result
    #[test]
    fn all_stopwords_returns_empty() {
        let entries = vec![entry("lighting", "control lights", &[])];
        assert_eq!(detect("the a and", entries), Vec::<String>::new());
    }

    // 11. Empty transcript → empty result
    #[test]
    fn empty_transcript_returns_empty() {
        let entries = vec![entry("lighting", "control lights", &[])];
        assert_eq!(detect("", entries), Vec::<String>::new());
    }

    // 12. Punctuation in transcript → stripped, still matches
    #[test]
    fn punctuation_stripped() {
        let entries = vec![entry("lighting", "control lights", &[])];
        assert_eq!(detect("lights, please.", entries), vec!["lighting"]);
    }

    // 13. Case insensitivity
    #[test]
    fn case_insensitive() {
        let entries = vec![entry("kitchen", "kitchen lights", &[])];
        assert_eq!(detect("KITCHEN", entries), vec!["kitchen"]);
    }

    // 14. Stop-word collision: capability named "the" → never matched
    #[test]
    fn stopword_capability_never_matched() {
        let entries = vec![entry("the", "definite article capability", &[])];
        // "the" is a stop-word, so tokenize drops it, so no match.
        assert_eq!(detect("the", entries), Vec::<String>::new());
    }

    // Additional sanity checks
    #[test]
    fn tokenize_basic() {
        // "on" and "the" are stop-words and get dropped.
        assert_eq!(
            tokenize("Turn on the kitchen lights!"),
            vec!["turn", "kitchen", "lights"]
        );
    }

    #[test]
    fn tokenize_strips_punctuation() {
        assert_eq!(tokenize("lights, please."), vec!["lights", "please"]);
    }

    #[test]
    fn capability_index_names() {
        let index = CapabilityIndex::from_entries(vec![
            entry("a", "desc a", &[]),
            entry("b", "desc b", &[]),
        ]);
        assert_eq!(index.names(), vec!["a", "b"]);
    }

    #[test]
    fn capability_index_default_is_empty() {
        assert!(CapabilityIndex::default().is_empty());
    }
}
