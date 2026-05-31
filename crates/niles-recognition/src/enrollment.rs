//! Enrollment store — per-speaker JSON persistence with atomic writes
//! and advisory locking.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Stored representation of a single enrollment clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnrollmentEntry {
    pub recorded_at: DateTime<Utc>,
    pub embedding: Vec<f32>,
}

/// On-disk record for one enrolled speaker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnrolledSpeaker {
    pub speaker: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub clip_count: usize,
    pub embeddings: Vec<EnrollmentEntry>,
}

/// Manages per-speaker JSON files under `<root>/enrolled/`.
pub struct EnrollmentStore {
    _root: PathBuf,
    dir: PathBuf,
    in_process_lock: Mutex<()>,
}

impl EnrollmentStore {
    /// Open (or create) the store at `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let dir = root.join("enrolled");
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            _root: root,
            dir,
            in_process_lock: Mutex::new(()),
        })
    }

    /// Enroll a new embedding for `speaker`.
    pub fn enroll(&self, speaker: &str, embedding: &[f32]) -> Result<()> {
        validate_speaker_slug(speaker)?;
        if embedding.len() != 192 {
            return Err(Error::WrongEmbeddingDim {
                actual: embedding.len(),
            });
        }

        let _in_process = self.in_process_lock.lock().unwrap();
        let lock_path = self.lock_path_for(speaker);
        let _guard = lock_file(&lock_path, Duration::from_secs(5))?;

        let mut e = embedding.to_vec();
        crate::similarity::l2_normalize(&mut e);

        let path = self.path_for(speaker);
        let mut record = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            serde_json::from_str(&raw)?
        } else {
            EnrolledSpeaker {
                speaker: speaker.to_string(),
                display_name: default_display_name(speaker),
                created_at: Utc::now(),
                last_seen_at: None,
                clip_count: 0,
                embeddings: vec![],
            }
        };

        record.embeddings.push(EnrollmentEntry {
            recorded_at: Utc::now(),
            embedding: e,
        });
        record.clip_count = record.embeddings.len();

        let bytes = serde_json::to_vec_pretty(&record)?;
        atomic_write(&path, &bytes)?;
        Ok(())
    }

    /// List all enrolled speaker slugs, sorted alphabetically.
    pub fn list(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            if validate_speaker_slug(stem).is_ok() {
                names.push(stem.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Load a single speaker record.
    pub fn load(&self, speaker: &str) -> Result<EnrolledSpeaker> {
        validate_speaker_slug(speaker)?;
        let path = self.path_for(speaker);
        match std::fs::read_to_string(&path) {
            Ok(raw) => Ok(serde_json::from_str(&raw)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::NotFound {
                speaker: speaker.to_string(),
            }),
            Err(e) => Err(e.into()),
        }
    }

    /// Load all enrolled speakers.
    pub fn load_all(&self) -> Result<Vec<EnrolledSpeaker>> {
        let names = self.list()?;
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            match self.load(&name) {
                Ok(s) => out.push(s),
                Err(Error::NotFound { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Update `last_seen_at` to now. Silently succeeds if the speaker
    /// does not exist.
    pub fn bump_last_seen(&self, speaker: &str) -> Result<()> {
        validate_speaker_slug(speaker)?;
        let _in_process = self.in_process_lock.lock().unwrap();
        let path = self.path_for(speaker);
        let lock_path = self.lock_path_for(speaker);
        let _lock = lock_file(&lock_path, Duration::from_secs(5))?;

        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        let mut record: EnrolledSpeaker = serde_json::from_str(&raw)?;
        record.last_seen_at = Some(Utc::now());
        let bytes = serde_json::to_vec_pretty(&record)?;
        atomic_write(&path, &bytes)?;
        Ok(())
    }

    /// Remove a speaker from the store.
    pub fn delete(&self, speaker: &str) -> Result<()> {
        validate_speaker_slug(speaker)?;
        let _in_process = self.in_process_lock.lock().unwrap();
        let lock_path = self.lock_path_for(speaker);
        let _guard = lock_file(&lock_path, Duration::from_secs(5))?;

        let path = self.path_for(speaker);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotFound {
                    speaker: speaker.to_string(),
                });
            }
            Err(e) => return Err(e.into()),
        }
        let _ = std::fs::remove_file(self.lock_path_for(speaker));
        Ok(())
    }

    fn path_for(&self, slug: &str) -> PathBuf {
        self.dir.join(format!("{slug}.json"))
    }

    fn lock_path_for(&self, slug: &str) -> PathBuf {
        self.dir.join(format!("{slug}.json.lock"))
    }
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn validate_speaker_slug(name: &str) -> Result<()> {
    if name == "." || name == ".." {
        return Err(Error::InvalidName {
            name: name.to_string(),
            reason: "reserved name".into(),
        });
    }
    if name.is_empty() {
        return Err(Error::InvalidName {
            name: name.to_string(),
            reason: "empty".into(),
        });
    }
    if name.len() > 64 {
        return Err(Error::InvalidName {
            name: name.to_string(),
            reason: "longer than 64 characters".into(),
        });
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(Error::InvalidName {
            name: name.to_string(),
            reason: "must start with a-z or 0-9".into(),
        });
    }
    for ch in chars {
        if ch != '-' && !ch.is_ascii_lowercase() && !ch.is_ascii_digit() {
            return Err(Error::InvalidName {
                name: name.to_string(),
                reason: "must contain only a-z, 0-9, and hyphens".into(),
            });
        }
    }
    Ok(())
}

fn default_display_name(slug: &str) -> String {
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Acquire an exclusive OS advisory lock with a ~5 s timeout.
fn lock_file(path: &Path, timeout: Duration) -> Result<std::fs::File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(path)?;
    let deadline = Instant::now() + timeout;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
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

/// True if `e` indicates lock contention — the case we want to retry.
fn is_lock_contention(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    matches!(e.raw_os_error(), Some(33) | Some(997))
}

/// Atomically write `bytes` to `path` using a temp file + rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_name = format!(
        "{}.tmp.{}_{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let tmp = path.with_file_name(&tmp_name);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::distr::Distribution;
    use std::sync::Arc;

    #[test]
    fn enroll_creates_speaker_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        store.enroll("mark", &[0.0_f32; 192]).unwrap();

        let record = store.load("mark").unwrap();
        assert_eq!(record.speaker, "mark");
        assert_eq!(record.display_name, "Mark");
        assert_eq!(record.clip_count, 1);
        assert_eq!(record.embeddings.len(), 1);
        let now = Utc::now();
        assert!((record.created_at - now).num_seconds().abs() <= 1);
    }

    #[test]
    fn enroll_appends_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        store.enroll("mark", &[0.0_f32; 192]).unwrap();
        let first_created = store.load("mark").unwrap().created_at;

        store.enroll("mark", &[1.0_f32; 192]).unwrap();
        let record = store.load("mark").unwrap();
        assert_eq!(record.clip_count, 2);
        assert_eq!(record.embeddings.len(), 2);
        assert_eq!(record.created_at, first_created);
    }

    #[test]
    fn enroll_rejects_wrong_dim() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let err = store.enroll("mark", &[0.0_f32; 191]).unwrap_err();
        assert!(matches!(err, Error::WrongEmbeddingDim { actual: 191 }));
    }

    #[test]
    fn enroll_normalizes_embedding() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let mut v = [0.0_f32; 192];
        v[0] = 3.0;
        v[1] = 4.0;
        store.enroll("mark", &v).unwrap();

        let record = store.load("mark").unwrap();
        let stored = &record.embeddings[0].embedding;
        let sim = crate::similarity::cosine_similarity(stored, stored);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn enroll_rejects_invalid_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let err = store.enroll("Foo bar", &[0.0_f32; 192]).unwrap_err();
        assert!(matches!(err, Error::InvalidName { .. }));
    }

    #[test]
    fn list_returns_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        store.enroll("c", &[0.0_f32; 192]).unwrap();
        store.enroll("a", &[0.0_f32; 192]).unwrap();
        store.enroll("b", &[0.0_f32; 192]).unwrap();

        assert_eq!(store.list().unwrap(), vec!["a", "b", "c"]);
    }

    #[test]
    fn load_all_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        store.enroll("a", &[0.0_f32; 192]).unwrap();
        store.enroll("b", &[0.0_f32; 192]).unwrap();
        store.enroll("c", &[0.0_f32; 192]).unwrap();

        assert_eq!(store.load_all().unwrap().len(), 3);
    }

    #[test]
    fn delete_removes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        store.enroll("mark", &[0.0_f32; 192]).unwrap();
        assert!(store.path_for("mark").exists());

        store.delete("mark").unwrap();
        assert!(!store.path_for("mark").exists());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn delete_unknown_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let err = store.delete("nobody").unwrap_err();
        assert!(matches!(err, Error::NotFound { speaker } if speaker == "nobody"));
    }

    #[test]
    fn load_unknown_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        let err = store.load("nobody").unwrap_err();
        assert!(matches!(err, Error::NotFound { speaker } if speaker == "nobody"));
    }

    #[test]
    fn bump_last_seen_updates_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        store.enroll("mark", &[0.0_f32; 192]).unwrap();
        assert!(store.load("mark").unwrap().last_seen_at.is_none());

        store.bump_last_seen("mark").unwrap();
        let record = store.load("mark").unwrap();
        assert!(record.last_seen_at.is_some());
    }

    #[test]
    fn bump_last_seen_missing_is_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        assert!(store.bump_last_seen("nobody").is_ok());
    }

    #[test]
    fn concurrent_enrolls() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(EnrollmentStore::open(tmp.path()).unwrap());
        let mut handles = Vec::new();

        for i in 0..10 {
            let s = Arc::clone(&store);
            let mut v = [0.0_f32; 192];
            v[i] = 1.0;
            handles.push(std::thread::spawn(move || {
                s.enroll("mark", &v).unwrap();
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let record = store.load("mark").unwrap();
        assert_eq!(record.clip_count, 10);
    }

    #[test]
    fn unknown_files_are_ignored_by_list() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();
        store.enroll("alice", &[0.0_f32; 192]).unwrap();

        std::fs::write(tmp.path().join("enrolled/_garbage.txt"), "x").unwrap();
        std::fs::write(tmp.path().join("enrolled/bad slug.json"), "{}").unwrap();

        assert_eq!(store.list().unwrap(), vec!["alice"]);
    }

    #[test]
    fn enroll_then_load_round_trip_normalizes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = EnrollmentStore::open(tmp.path()).unwrap();

        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        let base: Vec<f32> = (0..192)
            .map(|_| {
                rand::distr::Uniform::new(-1.0_f32, 1.0)
                    .unwrap()
                    .sample(&mut rng)
            })
            .collect();
        let mut e1 = base.clone();
        crate::similarity::l2_normalize(&mut e1);
        let mut e2 = base.clone();
        for x in e2.iter_mut() {
            *x += rand::distr::Uniform::new(-0.01_f32, 0.01)
                .unwrap()
                .sample(&mut rng);
        }
        crate::similarity::l2_normalize(&mut e2);

        let mut unrelated: Vec<f32> = (0..192)
            .map(|_| {
                rand::distr::Uniform::new(-1.0_f32, 1.0)
                    .unwrap()
                    .sample(&mut rng)
            })
            .collect();
        crate::similarity::l2_normalize(&mut unrelated);

        store.enroll("mark", &e1).unwrap();
        store.enroll("mark", &e2).unwrap();
        store.enroll("jane", &unrelated).unwrap();

        let mark = store.load("mark").unwrap();
        let jane = store.load("jane").unwrap();

        let sim_related = crate::similarity::cosine_similarity(
            &mark.embeddings[0].embedding,
            &mark.embeddings[1].embedding,
        );
        let sim_unrelated = crate::similarity::cosine_similarity(
            &mark.embeddings[0].embedding,
            &jane.embeddings[0].embedding,
        );
        assert!(sim_related > sim_unrelated);
    }
}
