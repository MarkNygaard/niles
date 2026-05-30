//! Persistent memory store: USER.md + MEMORY.md.
//!
//! File format: §-delimited markdown entries.  Atomic writes via
//! tempfile-rename + OS advisory file lock.

use crate::error::{Error, Result};
use crate::scan;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Which memory file to operate on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    User,
    Memory,
}

impl Target {
    fn file_name(self) -> &'static str {
        match self {
            Target::User => "USER.md",
            Target::Memory => "MEMORY.md",
        }
    }

    fn as_str(self) -> &'static str {
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
    pub directory: PathBuf,
    pub user_char_limit: usize,
    pub agent_char_limit: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("."),
            user_char_limit: 1375,
            agent_char_limit: 2200,
        }
    }
}

/// Persistent markdown memory store.
pub struct MemoryStore {
    config: MemoryConfig,
    enabled: bool,
}

impl MemoryStore {
    fn validate_entry_content(content: &str) -> Result<()> {
        if content.lines().any(|line| line.trim() == "§") {
            return Err(Error::InvalidDelimiter);
        }
        Ok(())
    }

    /// Open (or create) the memory store.  Missing `USER.md` and
    /// `MEMORY.md` files are created as empty.
    pub fn open(config: MemoryConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.directory)?;
        for target in [Target::User, Target::Memory] {
            let path = config.directory.join(target.file_name());
            if !path.exists() {
                File::create(&path)?;
            }
        }
        Ok(Self {
            config,
            enabled: true,
        })
    }

    /// Whether the store is enabled (has a backing directory).
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// No-op store for when memory is disabled.
    pub fn disabled() -> Self {
        Self {
            config: MemoryConfig {
                directory: PathBuf::new(),
                ..Default::default()
            },
            enabled: false,
        }
    }

    /// Read and parse entries from `target`.
    ///
    /// Returns an error if the file cannot be read or if a security
    /// scan of the raw content fails.
    pub fn load(&self, target: Target) -> Result<Vec<Entry>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let path = self.config.directory.join(target.file_name());
        let mut file = File::open(&path)?;
        let mut raw = String::new();
        file.read_to_string(&mut raw)?;
        scan::scan(&raw)?;
        Ok(parse_entries(&raw))
    }

    /// Append a new entry to `target`.  Enforces scan + budget.
    pub fn add(&self, target: Target, content: &str) -> Result<()> {
        if !self.enabled {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "memory store is disabled",
            )));
        }
        scan::scan(content)?;
        Self::validate_entry_content(content)?;
        let limit = self.char_limit(target);
        self.mutate_file(target, |raw| {
            let mut entries = parse_entries(raw);
            entries.push(Entry {
                text: content.trim().to_string(),
            });
            let out = serialize_entries(&entries);
            if out.chars().count() > limit {
                return Err(Error::OverBudget {
                    target: target.as_str(),
                    limit,
                    actual: out.chars().count(),
                });
            }
            Ok(out)
        })
    }

    /// Replace the first entry whose text contains `old_text` with
    /// `new_content`.  Errors if no match or ambiguous.
    pub fn replace(&self, target: Target, old_text: &str, new_content: &str) -> Result<()> {
        if old_text.is_empty() {
            return Err(Error::EmptySearch);
        }
        if !self.enabled {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "memory store is disabled",
            )));
        }
        scan::scan(new_content)?;
        Self::validate_entry_content(new_content)?;
        let limit = self.char_limit(target);
        self.mutate_file(target, |raw| {
            let entries = parse_entries(raw);
            let matches: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.text.contains(old_text))
                .map(|(i, _)| i)
                .collect();
            if matches.is_empty() {
                return Err(Error::NotFound {
                    target: target.as_str(),
                });
            }
            if matches.len() > 1 {
                return Err(Error::Ambiguous {
                    target: target.as_str(),
                });
            }
            let mut entries = entries;
            entries[matches[0]] = Entry {
                text: new_content.trim().to_string(),
            };
            // deduplicate after replacement
            let out = serialize_entries(&entries);
            if out.chars().count() > limit {
                return Err(Error::OverBudget {
                    target: target.as_str(),
                    limit,
                    actual: out.chars().count(),
                });
            }
            Ok(out)
        })
    }

    /// Remove the first entry whose text contains `old_text`.
    /// Errors if no match or ambiguous.
    pub fn remove(&self, target: Target, old_text: &str) -> Result<()> {
        if old_text.is_empty() {
            return Err(Error::EmptySearch);
        }
        if !self.enabled {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "memory store is disabled",
            )));
        }
        self.mutate_file(target, |raw| {
            let entries = parse_entries(raw);
            let matches: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.text.contains(old_text))
                .map(|(i, _)| i)
                .collect();
            if matches.is_empty() {
                return Err(Error::NotFound {
                    target: target.as_str(),
                });
            }
            if matches.len() > 1 {
                return Err(Error::Ambiguous {
                    target: target.as_str(),
                });
            }
            let mut entries = entries;
            entries.remove(matches[0]);
            Ok(serialize_entries(&entries))
        })
    }

    fn char_limit(&self, target: Target) -> usize {
        match target {
            Target::User => self.config.user_char_limit,
            Target::Memory => self.config.agent_char_limit,
        }
    }

    /// Acquire an exclusive OS file lock with a ~5 s timeout.
    fn lock_file(path: &Path) -> Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(path)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(file),
                Err(e) if is_lock_contention(&e) => {
                    if Instant::now() > deadline {
                        return Err(Error::Locked);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(Error::Io(e)),
            }
        }
    }

    /// Read → apply `f` → scan → atomic write.
    ///
    /// The closure `f` is responsible for budget checks (add/replace).
    /// Uses a separate `.lock` file so the advisory lock survives
    /// the rename of the target file.
    fn mutate_file(&self, target: Target, f: impl FnOnce(&str) -> Result<String>) -> Result<()> {
        let path = self.config.directory.join(target.file_name());
        let lock_path = path.with_extension("md.lock");

        // Acquire lock on the lock-file (created if absent).
        let _lock = Self::lock_file(&lock_path)?;

        let mut raw = String::new();
        {
            let mut file = File::open(&path)?;
            file.read_to_string(&mut raw)?;
        }

        let new_content = f(&raw)?;
        // scan already done in caller for add/replace; re-run for safety
        scan::scan(&new_content)?;

        let tmp_name = format!("{}.tmp.{}", target.file_name(), std::process::id());
        let tmp_path = path.with_file_name(&tmp_name);
        let result = (|| {
            {
                let mut tmp = File::create(&tmp_path)?;
                tmp.write_all(new_content.as_bytes())?;
                tmp.sync_all()?;
            }
            std::fs::rename(&tmp_path, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        result
    }
}

/// True if `e` indicates "the lock is held by another process / thread" —
/// the case we want to retry, not propagate.
///
/// On Linux, `try_lock_exclusive` reports this via
/// `ErrorKind::WouldBlock`. On Windows, `LockFileEx` with
/// `LOCKFILE_FAIL_IMMEDIATELY` instead surfaces as
/// `ErrorKind::Uncategorized` with raw OS error 33
/// (`ERROR_LOCK_VIOLATION`) — sometimes 997 (`ERROR_IO_PENDING`)
/// under async-style overlapped I/O. Without recognizing those, the
/// retry loop bails on the first contention on Windows.
fn is_lock_contention(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    matches!(e.raw_os_error(), Some(33) | Some(997))
}

/// Parse raw file content into deduplicated, trimmed entries.
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
    use tempfile::TempDir;

    fn setup() -> (TempDir, MemoryStore) {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        (tmp, store)
    }

    #[test]
    fn open_creates_missing_files() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        assert!(tmp.path().join("USER.md").exists());
        assert!(tmp.path().join("MEMORY.md").exists());
        let user = store.load(Target::User).unwrap();
        assert!(user.is_empty());
    }

    #[test]
    fn add_and_load_round_trip() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Alice likes tea.");
    }

    #[test]
    fn add_multiple_entries() {
        let (_tmp, store) = setup();
        store.add(Target::User, "First entry.").unwrap();
        store.add(Target::User, "Second entry.").unwrap();
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "First entry.");
        assert_eq!(entries[1].text, "Second entry.");
    }

    #[test]
    fn multiline_entry() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Line one.\nLine two.").unwrap();
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Line one.\nLine two.");
    }

    #[test]
    fn deduplication() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Duplicate.").unwrap();
        store.add(Target::User, "Duplicate.").unwrap();
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn whitespace_only_blocks_stripped() {
        let (_tmp, store) = setup();
        store.add(Target::User, "A").unwrap();
        store.add(Target::User, "   ").unwrap();
        store.add(Target::User, "B").unwrap();
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "A");
        assert_eq!(entries[1].text, "B");
    }

    #[test]
    fn replace_happy_path() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        store
            .replace(Target::User, "likes tea", "Alice likes coffee.")
            .unwrap();
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Alice likes coffee.");
    }

    #[test]
    fn replace_no_match_errors() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        let err = store
            .replace(Target::User, "likes beer", "likes coffee")
            .unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }));
    }

    #[test]
    fn replace_ambiguous_errors() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        store.add(Target::User, "Bob likes tea too.").unwrap();
        let err = store
            .replace(Target::User, "likes tea", "likes water")
            .unwrap_err();
        assert!(matches!(err, Error::Ambiguous { .. }));
    }

    #[test]
    fn remove_happy_path() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        store.remove(Target::User, "Alice").unwrap();
        let entries = store.load(Target::User).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn remove_no_match_errors() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        let err = store.remove(Target::User, "Bob").unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }));
    }

    #[test]
    fn remove_ambiguous_errors() {
        let (_tmp, store) = setup();
        store.add(Target::User, "Alice likes tea.").unwrap();
        store.add(Target::User, "Alice likes coffee.").unwrap();
        let err = store.remove(Target::User, "Alice").unwrap_err();
        assert!(matches!(err, Error::Ambiguous { .. }));
    }

    #[test]
    fn scan_blocks_bad_content() {
        let (_tmp, store) = setup();
        let err = store.add(Target::User, "hello\0world").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
    }

    #[test]
    fn over_budget_rejected() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            user_char_limit: 10,
            agent_char_limit: 10,
        })
        .unwrap();
        let err = store
            .add(Target::User, "This is way too long.")
            .unwrap_err();
        assert!(matches!(err, Error::OverBudget { .. }));
    }

    #[test]
    fn user_and_agent_are_separate() {
        let (_tmp, store) = setup();
        store.add(Target::User, "User fact.").unwrap();
        store.add(Target::Memory, "Agent learning.").unwrap();
        assert_eq!(store.load(Target::User).unwrap().len(), 1);
        assert_eq!(store.load(Target::Memory).unwrap().len(), 1);
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
        let out = serialize_entries(&entries);
        assert_eq!(out, "First entry.\n§\nSecond entry.");
    }

    #[test]
    fn parse_existing_file_format() {
        let tmp = TempDir::new().unwrap();
        let content = "First entry.\n§\nSecond entry.\n§\nThird entry.";
        std::fs::write(tmp.path().join("USER.md"), content).unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "First entry.");
        assert_eq!(entries[1].text, "Second entry.");
        assert_eq!(entries[2].text, "Third entry.");
    }

    #[test]
    fn concurrent_adds_are_atomic() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            user_char_limit: 100_000,
            agent_char_limit: 100_000,
        })
        .unwrap();
        let store = std::sync::Arc::new(store);
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
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 200);
    }

    #[test]
    fn is_enabled_true_for_open_store() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        assert!(store.is_enabled());
    }

    #[test]
    fn is_enabled_false_for_disabled_store() {
        let store = MemoryStore::disabled();
        assert!(!store.is_enabled());
    }

    #[test]
    fn disabled_store_returns_empty() {
        let store = MemoryStore::disabled();
        assert!(store.load(Target::User).unwrap().is_empty());
        assert!(store.load(Target::Memory).unwrap().is_empty());
        assert!(store.add(Target::User, "x").is_err());
        assert!(store.replace(Target::User, "x", "y").is_err());
        assert!(store.remove(Target::User, "x").is_err());
    }

    #[test]
    fn load_rejects_corrupted_file() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("USER.md"), "hello\0world").unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        let err = store.load(Target::User).unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }), "{err}");
    }

    #[test]
    fn exact_budget_is_allowed() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            user_char_limit: 5,
            agent_char_limit: 5,
        })
        .unwrap();
        // "hello" is exactly 5 chars
        store.add(Target::User, "hello").unwrap();
        let entries = store.load(Target::User).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "hello");
    }

    #[test]
    fn replace_over_budget_rejected() {
        let tmp = TempDir::new().unwrap();
        let store = MemoryStore::open(MemoryConfig {
            directory: tmp.path().to_path_buf(),
            user_char_limit: 10,
            agent_char_limit: 10,
        })
        .unwrap();
        store.add(Target::User, "short").unwrap();
        let err = store
            .replace(Target::User, "short", "way too long text")
            .unwrap_err();
        assert!(matches!(err, Error::OverBudget { .. }), "{err}");
    }

    #[test]
    fn replace_rejects_empty_old_text() {
        let (_tmp, store) = setup();
        store.add(Target::User, "A").unwrap();
        let err = store.replace(Target::User, "", "B").unwrap_err();
        assert!(matches!(err, Error::EmptySearch));
    }

    #[test]
    fn remove_rejects_empty_old_text() {
        let (_tmp, store) = setup();
        store.add(Target::User, "A").unwrap();
        let err = store.remove(Target::User, "").unwrap_err();
        assert!(matches!(err, Error::EmptySearch));
    }

    #[test]
    fn add_rejects_section_delimiter_line() {
        let (_tmp, store) = setup();
        let err = store.add(Target::User, "line 1\n§\nline 2").unwrap_err();
        assert!(matches!(err, Error::InvalidDelimiter));
    }

    #[test]
    fn replace_rejects_section_delimiter_line() {
        let (_tmp, store) = setup();
        store.add(Target::User, "A").unwrap();
        let err = store
            .replace(Target::User, "A", "line 1\n§\nline 2")
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDelimiter));
    }
}
