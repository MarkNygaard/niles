//! The memory documents: facts about the household, and what Niles
//! has learned.
//!
//! Format: §-delimited markdown entries. They live here, in memory, and
//! every change is handed to a [`MemoryPersistence`] — in Niles, a
//! Postgres row — so they outlast a restart without a directory to
//! configure.

use crate::error::{Error, Result};
use crate::scan;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

/// Which memory document to operate on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "agent")]
    Memory,
}

/// A note taken out of memory — by somebody, or by Niles tidying — and
/// kept a while so it can be put back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forgotten {
    pub target: Target,
    pub text: String,
    pub at: DateTime<Utc>,
}

/// How many forgotten notes are kept to put back.
const MAX_FORGOTTEN: usize = 50;

/// How alike two notes must be, as the share of their words they have in
/// common, to be the same note said twice. Three wordings of one
/// automation reminder, written in a minute, shared 0.61.
const SIMILAR: f64 = 0.6;

impl Target {
    pub fn as_str(self) -> &'static str {
        match self {
            Target::User => "user",
            Target::Memory => "agent",
        }
    }
}

/// In-memory representation of one loaded entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub text: String,
}

/// Configuration for a [`MemoryStore`].
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub user_char_limit: usize,
    pub agent_char_limit: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            user_char_limit: 1375,
            agent_char_limit: 2200,
        }
    }
}

/// Where a changed document goes to be kept. Called with the lock held,
/// so documents arrive in the order they were written; it should only
/// hand the document on, not wait for it to be stored.
pub trait MemoryPersistence: Send + Sync {
    fn store(&self, target: Target, document: &str);
    /// The forgotten notes, as JSON.
    fn store_forgotten(&self, document: &str);
}

#[derive(Default)]
struct Documents {
    user: String,
    agent: String,
    forgotten: Vec<Forgotten>,
}

impl Documents {
    fn get_mut(&mut self, target: Target) -> &mut String {
        match target {
            Target::User => &mut self.user,
            Target::Memory => &mut self.agent,
        }
    }
}

/// The household's memory.
pub struct MemoryStore {
    config: MemoryConfig,
    documents: Mutex<Documents>,
    sink: Option<Arc<dyn MemoryPersistence>>,
}

impl MemoryStore {
    /// Empty, kept only as long as the process.
    pub fn new(config: MemoryConfig) -> Self {
        Self {
            config,
            documents: Mutex::new(Documents::default()),
            sink: None,
        }
    }

    /// Start from documents kept earlier. Refused when either fails the
    /// security scan: memory goes into every prompt.
    pub fn with_documents(self, user: &str, agent: &str) -> Result<Self> {
        scan::scan(user)?;
        scan::scan(agent)?;
        let mut documents = self.lock();
        documents.user = user.to_string();
        documents.agent = agent.to_string();
        drop(documents);
        Ok(self)
    }

    /// Start from the notes forgotten earlier.
    pub fn with_forgotten(self, document: &str) -> Result<Self> {
        let forgotten: Vec<Forgotten> = serde_json::from_str(document)?;
        for f in &forgotten {
            scan::scan(&f.text)?;
        }
        self.lock().forgotten = forgotten;
        Ok(self)
    }

    /// Hand every change to `sink`.
    pub fn with_sink(mut self, sink: Arc<dyn MemoryPersistence>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn lock(&self) -> MutexGuard<'_, Documents> {
        self.documents.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn validate_entry_content(content: &str) -> Result<()> {
        if content.lines().any(|line| line.trim() == "§") {
            return Err(Error::InvalidDelimiter);
        }
        Ok(())
    }

    /// The entries in `target`.
    pub fn load(&self, target: Target) -> Vec<Entry> {
        parse_entries(self.lock().get_mut(target))
    }

    /// How full `target` is: characters used, and the most it holds.
    pub fn usage(&self, target: Target) -> (usize, usize) {
        let used = self.lock().get_mut(target).chars().count();
        (used, self.char_limit(target))
    }

    /// The notes forgotten lately, newest first.
    pub fn forgotten(&self) -> Vec<Forgotten> {
        self.lock().forgotten.iter().rev().cloned().collect()
    }

    /// Append a new entry to `target`. Enforces scan + budget, and
    /// refuses a note that is one already there said again.
    pub fn add(&self, target: Target, content: &str) -> Result<()> {
        scan::scan(content)?;
        Self::validate_entry_content(content)?;
        let limit = self.char_limit(target);
        self.mutate(target, |raw| {
            let mut entries = parse_entries(raw);
            if let Some(existing) = entries.iter().find(|e| similar(&e.text, content)) {
                return Err(Error::Similar {
                    target: target.as_str(),
                    existing: existing.text.clone(),
                });
            }
            entries.push(Entry {
                text: content.trim().to_string(),
            });
            Ok((
                within_budget(target, limit, serialize_entries(&entries))?,
                None,
            ))
        })
    }

    /// Put a forgotten note back.
    pub fn restore(&self, target: Target, text: &str) -> Result<()> {
        let limit = self.char_limit(target);
        let mut documents = self.lock();
        let at = documents
            .forgotten
            .iter()
            .rposition(|f| f.target == target && f.text == text)
            .ok_or(Error::NotForgotten {
                target: target.as_str(),
            })?;
        let mut entries = parse_entries(documents.get_mut(target));
        entries.push(Entry {
            text: text.to_string(),
        });
        let restored = within_budget(target, limit, serialize_entries(&entries))?;
        *documents.get_mut(target) = restored;
        documents.forgotten.remove(at);
        self.keep(&mut documents, target, true);
        Ok(())
    }

    /// Replace the entry whose text contains `old_text` with
    /// `new_content`.  Errors if no match or ambiguous.
    pub fn replace(&self, target: Target, old_text: &str, new_content: &str) -> Result<()> {
        if old_text.is_empty() {
            return Err(Error::EmptySearch);
        }
        scan::scan(new_content)?;
        Self::validate_entry_content(new_content)?;
        let limit = self.char_limit(target);
        self.mutate(target, |raw| {
            let mut entries = parse_entries(raw);
            let at = only_match(target, &entries, old_text)?;
            let old = std::mem::replace(
                &mut entries[at],
                Entry {
                    text: new_content.trim().to_string(),
                },
            );
            Ok((
                within_budget(target, limit, serialize_entries(&entries))?,
                Some(old.text),
            ))
        })
    }

    /// Remove the entry whose text contains `old_text`.
    /// Errors if no match or ambiguous.
    pub fn remove(&self, target: Target, old_text: &str) -> Result<()> {
        if old_text.is_empty() {
            return Err(Error::EmptySearch);
        }
        self.mutate(target, |raw| {
            let mut entries = parse_entries(raw);
            let at = only_match(target, &entries, old_text)?;
            let gone = entries.remove(at);
            Ok((serialize_entries(&entries), Some(gone.text)))
        })
    }

    /// Remove the entry whose text is exactly `text` — for a person
    /// pointing at one, where "contains" could take out a longer entry
    /// that happens to include a short one, or refuse to choose.
    pub fn forget(&self, target: Target, text: &str) -> Result<()> {
        self.mutate(target, |raw| {
            let mut entries = parse_entries(raw);
            let at = entries
                .iter()
                .position(|e| e.text == text.trim())
                .ok_or(Error::NotFound {
                    target: target.as_str(),
                })?;
            let gone = entries.remove(at);
            Ok((serialize_entries(&entries), Some(gone.text)))
        })
    }

    fn char_limit(&self, target: Target) -> usize {
        match target {
            Target::User => self.config.user_char_limit,
            Target::Memory => self.config.agent_char_limit,
        }
    }

    /// Read → apply `f` → scan → keep, and hand the result on. What `f`
    /// says it took out is kept among the forgotten notes.
    fn mutate(
        &self,
        target: Target,
        f: impl FnOnce(&str) -> Result<(String, Option<String>)>,
    ) -> Result<()> {
        let mut documents = self.lock();
        let (changed, gone) = f(documents.get_mut(target))?;
        scan::scan(&changed)?;
        *documents.get_mut(target) = changed;
        if let Some(text) = &gone {
            documents.forgotten.push(Forgotten {
                target,
                text: text.clone(),
                at: Utc::now(),
            });
            if documents.forgotten.len() > MAX_FORGOTTEN {
                let excess = documents.forgotten.len() - MAX_FORGOTTEN;
                documents.forgotten.drain(..excess);
            }
        }
        self.keep(&mut documents, target, gone.is_some());
        Ok(())
    }

    fn keep(&self, documents: &mut Documents, target: Target, forgotten_changed: bool) {
        let Some(sink) = &self.sink else { return };
        sink.store(target, documents.get_mut(target));
        if forgotten_changed {
            match serde_json::to_string(&documents.forgotten) {
                Ok(document) => sink.store_forgotten(&document),
                Err(e) => tracing::error!("the forgotten notes could not be written down: {e}"),
            }
        }
    }
}

/// Whether two notes are the same note in other words: most of their
/// words in common. Short words ("a", "to", "in") say nothing either way.
fn similar(a: &str, b: &str) -> bool {
    let words = |s: &str| -> HashSet<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() > 2)
            .map(str::to_string)
            .collect()
    };
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let shared = a.intersection(&b).count() as f64;
    shared / a.union(&b).count() as f64 >= SIMILAR
}

fn only_match(target: Target, entries: &[Entry], old_text: &str) -> Result<usize> {
    let mut matches = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.text.contains(old_text))
        .map(|(i, _)| i);
    match (matches.next(), matches.next()) {
        (Some(at), None) => Ok(at),
        (None, _) => Err(Error::NotFound {
            target: target.as_str(),
        }),
        (Some(_), Some(_)) => Err(Error::Ambiguous {
            target: target.as_str(),
        }),
    }
}

fn within_budget(target: Target, limit: usize, document: String) -> Result<String> {
    let actual = document.chars().count();
    if actual > limit {
        return Err(Error::OverBudget {
            target: target.as_str(),
            limit,
            actual,
        });
    }
    Ok(document)
}

/// Parse a document into deduplicated, trimmed entries.
fn parse_entries(raw: &str) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for block in raw.split("\n§\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() || !seen.insert(trimmed) {
            continue;
        }
        out.push(Entry {
            text: trimmed.to_string(),
        });
    }
    out
}

/// Serialize entries back to the §-delimited format.
fn serialize_entries(entries: &[Entry]) -> String {
    let mut out = String::new();
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&e.text);
        if i + 1 < entries.len() {
            out.push_str("\n§\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> MemoryStore {
        MemoryStore::new(MemoryConfig::default())
    }

    fn limited(limit: usize) -> MemoryStore {
        MemoryStore::new(MemoryConfig {
            user_char_limit: limit,
            agent_char_limit: limit,
        })
    }

    #[derive(Default)]
    struct Kept(Mutex<Vec<(Target, String)>>, Mutex<Vec<String>>);

    impl MemoryPersistence for Kept {
        fn store(&self, target: Target, document: &str) {
            self.0.lock().unwrap().push((target, document.to_string()));
        }

        fn store_forgotten(&self, document: &str) {
            self.1.lock().unwrap().push(document.to_string());
        }
    }

    #[test]
    fn a_new_store_is_empty() {
        let store = setup();
        assert!(store.load(Target::User).is_empty());
        assert!(store.load(Target::Memory).is_empty());
    }

    #[test]
    fn add_and_load_round_trip() {
        let store = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        let entries = store.load(Target::User);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Alice likes tea.");
    }

    #[test]
    fn add_multiple_entries() {
        let store = setup();
        store.add(Target::User, "First entry.").unwrap();
        store.add(Target::User, "Second entry.").unwrap();
        let entries = store.load(Target::User);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "First entry.");
        assert_eq!(entries[1].text, "Second entry.");
    }

    #[test]
    fn multiline_entry() {
        let store = setup();
        store.add(Target::User, "Line one.\nLine two.").unwrap();
        let entries = store.load(Target::User);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Line one.\nLine two.");
    }

    #[test]
    fn deduplication() {
        let store = setup();
        store.add(Target::User, "Duplicate.").unwrap();
        let err = store.add(Target::User, "Duplicate.").unwrap_err();
        assert!(matches!(err, Error::Similar { .. }), "{err}");
        assert_eq!(store.load(Target::User).len(), 1);
    }

    /// The three notes the review wrote on 2026-10-13, a minute apart.
    #[test]
    fn the_same_note_in_other_words_is_refused() {
        let store = setup();
        store
            .add(
                Target::Memory,
                "User wants an automation that triggers 30 minutes after sunset to notify \
                 \"Please close the curtains\" in the living room.",
            )
            .unwrap();
        let err = store
            .add(
                Target::Memory,
                "User wants automation that triggers 30 minutes after sunset to perform an \
                 action (e.g., notify to close curtains).",
            )
            .unwrap_err();
        assert!(
            matches!(err, Error::Similar { ref existing, .. } if existing.contains("living room")),
            "{err}"
        );
        // Different facts that share a few words are both kept.
        store
            .add(Target::User, "Mark takes his coffee black.")
            .unwrap();
        store
            .add(Target::User, "Majse takes her coffee with oat milk.")
            .unwrap();
        assert_eq!(store.load(Target::User).len(), 2);
    }

    #[test]
    fn what_is_taken_out_can_be_put_back() {
        let kept = Arc::new(Kept::default());
        let store = setup().with_sink(kept.clone());
        store
            .add(Target::User, "Mark takes his coffee black.")
            .unwrap();
        store
            .replace(Target::User, "coffee", "Mark takes his coffee with milk.")
            .unwrap();
        store
            .forget(Target::User, "Mark takes his coffee with milk.")
            .unwrap();
        let forgotten: Vec<_> = store.forgotten().into_iter().map(|f| f.text).collect();
        assert_eq!(
            forgotten,
            vec![
                "Mark takes his coffee with milk.",
                "Mark takes his coffee black."
            ]
        );
        assert!(store.load(Target::User).is_empty());

        store
            .restore(Target::User, "Mark takes his coffee black.")
            .unwrap();
        assert_eq!(
            store.load(Target::User)[0].text,
            "Mark takes his coffee black."
        );
        assert_eq!(store.forgotten().len(), 1);
        assert!(matches!(
            store.restore(Target::User, "never said").unwrap_err(),
            Error::NotForgotten { .. }
        ));

        let next = setup()
            .with_forgotten(&kept.1.lock().unwrap().last().cloned().unwrap())
            .unwrap();
        assert_eq!(next.forgotten().len(), 1);
    }

    #[test]
    fn says_how_full_it_is() {
        let store = limited(100);
        store
            .add(Target::User, "Mark takes his coffee black.")
            .unwrap();
        assert_eq!(store.usage(Target::User), (28, 100));
    }

    #[test]
    fn whitespace_only_blocks_stripped() {
        let store = setup();
        store.add(Target::User, "A").unwrap();
        store.add(Target::User, "   ").unwrap();
        store.add(Target::User, "B").unwrap();
        let entries = store.load(Target::User);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "A");
        assert_eq!(entries[1].text, "B");
    }

    #[test]
    fn replace_happy_path() {
        let store = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        store
            .replace(Target::User, "likes tea", "Alice likes coffee.")
            .unwrap();
        let entries = store.load(Target::User);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Alice likes coffee.");
    }

    #[test]
    fn replace_no_match_errors() {
        let store = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        let err = store
            .replace(Target::User, "likes beer", "likes coffee")
            .unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }));
    }

    #[test]
    fn replace_ambiguous_errors() {
        let store = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        store.add(Target::User, "Bob likes tea too.").unwrap();
        let err = store
            .replace(Target::User, "likes tea", "likes water")
            .unwrap_err();
        assert!(matches!(err, Error::Ambiguous { .. }));
    }

    #[test]
    fn remove_happy_path() {
        let store = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        store.remove(Target::User, "Alice").unwrap();
        assert!(store.load(Target::User).is_empty());
    }

    #[test]
    fn remove_no_match_errors() {
        let store = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        let err = store.remove(Target::User, "Bob").unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }));
    }

    #[test]
    fn remove_ambiguous_errors() {
        let store = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        store.add(Target::User, "Alice likes coffee.").unwrap();
        let err = store.remove(Target::User, "Alice").unwrap_err();
        assert!(matches!(err, Error::Ambiguous { .. }));
    }

    #[test]
    fn forgetting_takes_exactly_the_entry_named() {
        // Kept earlier, before notes this alike were refused.
        let store = setup()
            .with_documents(
                "Mark takes his coffee black\n§\nMark takes his coffee black on weekdays",
                "",
            )
            .unwrap();
        store
            .forget(Target::User, "Mark takes his coffee black")
            .unwrap();
        assert_eq!(
            store.load(Target::User),
            vec![Entry {
                text: "Mark takes his coffee black on weekdays".into()
            }]
        );
        let err = store.forget(Target::User, "coffee").unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }));
    }

    #[test]
    fn scan_blocks_bad_content() {
        let store = setup();
        let err = store.add(Target::User, "hello\0world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
    }

    #[test]
    fn over_budget_rejected() {
        let err = limited(10)
            .add(Target::User, "This is way too long.")
            .unwrap_err();
        assert!(matches!(err, Error::OverBudget { .. }));
    }

    #[test]
    fn exact_budget_is_allowed() {
        let store = limited(5);
        store.add(Target::User, "hello").unwrap();
        assert_eq!(store.load(Target::User)[0].text, "hello");
    }

    #[test]
    fn replace_over_budget_rejected() {
        let store = limited(10);
        store.add(Target::User, "short").unwrap();
        let err = store
            .replace(Target::User, "short", "way too long text")
            .unwrap_err();
        assert!(matches!(err, Error::OverBudget { .. }), "{err}");
        assert_eq!(store.load(Target::User)[0].text, "short");
    }

    #[test]
    fn user_and_agent_are_separate() {
        let store = setup();
        store.add(Target::User, "User fact.").unwrap();
        store.add(Target::Memory, "Agent learning.").unwrap();
        assert_eq!(store.load(Target::User).len(), 1);
        assert_eq!(store.load(Target::Memory).len(), 1);
    }

    #[test]
    fn serialization_format() {
        let entries = vec![
            Entry {
                text: "First entry.".into(),
            },
            Entry {
                text: "Second entry.".into(),
            },
        ];
        assert_eq!(
            serialize_entries(&entries),
            "First entry.\n§\nSecond entry."
        );
    }

    #[test]
    fn starts_from_kept_documents() {
        let store = setup()
            .with_documents(
                "First entry.\n§\nSecond entry.\n§\nThird entry.",
                "Learned.",
            )
            .unwrap();
        let entries = store.load(Target::User);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[2].text, "Third entry.");
        assert_eq!(store.load(Target::Memory)[0].text, "Learned.");
    }

    #[test]
    fn refuses_kept_documents_that_fail_the_scan() {
        let err = setup().with_documents("hello\0world", "").err().unwrap();
        assert!(matches!(err, Error::ScanFailed { .. }), "{err}");
    }

    #[test]
    fn every_change_is_handed_on_whole() {
        let kept = Arc::new(Kept::default());
        let store = setup().with_sink(kept.clone());
        store.add(Target::User, "Alice likes tea.").unwrap();
        store.add(Target::Memory, "Mornings are quiet.").unwrap();
        store.add(Target::User, "Bob likes coffee.").unwrap();
        store.remove(Target::User, "Alice").unwrap();
        let kept = kept.0.lock().unwrap();
        assert_eq!(
            *kept,
            vec![
                (Target::User, "Alice likes tea.".to_string()),
                (Target::Memory, "Mornings are quiet.".to_string()),
                (
                    Target::User,
                    "Alice likes tea.\n§\nBob likes coffee.".to_string()
                ),
                (Target::User, "Bob likes coffee.".to_string()),
            ]
        );
    }

    #[test]
    fn a_refused_change_is_not_handed_on() {
        let kept = Arc::new(Kept::default());
        let store = limited(10).with_sink(kept.clone());
        let _ = store.add(Target::User, "This is way too long.");
        let _ = store.remove(Target::User, "nothing");
        assert!(kept.0.lock().unwrap().is_empty());
    }

    #[test]
    fn concurrent_adds_are_atomic() {
        let store = Arc::new(limited(100_000));
        let mut handles = Vec::new();
        for t in 0..10 {
            let s = store.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..20 {
                    s.add(Target::User, &format!("entry{t}x{i}")).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(store.load(Target::User).len(), 200);
    }

    #[test]
    fn replace_rejects_empty_old_text() {
        let store = setup();
        store.add(Target::User, "A").unwrap();
        let err = store.replace(Target::User, "", "B").unwrap_err();
        assert!(matches!(err, Error::EmptySearch));
    }

    #[test]
    fn remove_rejects_empty_old_text() {
        let store = setup();
        store.add(Target::User, "A").unwrap();
        let err = store.remove(Target::User, "").unwrap_err();
        assert!(matches!(err, Error::EmptySearch));
    }

    #[test]
    fn add_rejects_section_delimiter_line() {
        let err = setup().add(Target::User, "line 1\n§\nline 2").unwrap_err();
        assert!(matches!(err, Error::InvalidDelimiter));
    }

    #[test]
    fn replace_rejects_section_delimiter_line() {
        let store = setup();
        store.add(Target::User, "A").unwrap();
        let err = store
            .replace(Target::User, "A", "line 1\n§\nline 2")
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDelimiter));
    }
}
