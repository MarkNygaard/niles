//! The memory documents: facts about the household, and what Niles
//! has learned.
//!
//! Format: §-delimited markdown entries. They live here, in memory, and
//! every change is handed to a [`MemoryPersistence`] — in Niles, a
//! Postgres row — so they outlast a restart without a directory to
//! configure.

use crate::error::{Error, Result};
use crate::scan;
use std::sync::{Arc, Mutex, MutexGuard};

/// Which memory document to operate on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    User,
    Memory,
}

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
}

#[derive(Default)]
struct Documents {
    user: String,
    agent: String,
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
        *self.lock() = Documents {
            user: user.to_string(),
            agent: agent.to_string(),
        };
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

    /// Append a new entry to `target`.  Enforces scan + budget.
    pub fn add(&self, target: Target, content: &str) -> Result<()> {
        scan::scan(content)?;
        Self::validate_entry_content(content)?;
        let limit = self.char_limit(target);
        self.mutate(target, |raw| {
            let mut entries = parse_entries(raw);
            entries.push(Entry {
                text: content.trim().to_string(),
            });
            within_budget(target, limit, serialize_entries(&entries))
        })
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
            entries[at] = Entry {
                text: new_content.trim().to_string(),
            };
            within_budget(target, limit, serialize_entries(&entries))
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
            entries.remove(at);
            Ok(serialize_entries(&entries))
        })
    }

    fn char_limit(&self, target: Target) -> usize {
        match target {
            Target::User => self.config.user_char_limit,
            Target::Memory => self.config.agent_char_limit,
        }
    }

    /// Read → apply `f` → scan → keep, and hand the result on.
    fn mutate(&self, target: Target, f: impl FnOnce(&str) -> Result<String>) -> Result<()> {
        let mut documents = self.lock();
        let document = documents.get_mut(target);
        let changed = f(document)?;
        scan::scan(&changed)?;
        *document = changed;
        if let Some(sink) = &self.sink {
            sink.store(target, document);
        }
        Ok(())
    }
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
    struct Kept(Mutex<Vec<(Target, String)>>);

    impl MemoryPersistence for Kept {
        fn store(&self, target: Target, document: &str) {
            self.0.lock().unwrap().push((target, document.to_string()));
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
        store.add(Target::User, "Duplicate.").unwrap();
        assert_eq!(store.load(Target::User).len(), 1);
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
                    s.add(Target::User, &format!("thread-{t}-entry-{i}"))
                        .unwrap();
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
