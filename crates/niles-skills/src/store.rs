//! SkillStore — atomic write-side storage for agent-mintable skills.

use std::fs::{OpenOptions, read_dir};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::scan;
use crate::sidecar::{Provenance, Sidecar, SkillStatus};
use crate::util::atomic_write;

/// A loaded skill with metadata, body, and telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub version: String,
    pub body: String,
    pub sidecar: Sidecar,
}

/// One-line metadata used to render the system-prompt
/// `Available skills` section without loading any body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub version: String,
    pub pinned: bool,
    pub provenance: Provenance,
    pub status: SkillStatus,
    pub last_activity_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillStatusFilter {
    /// Excludes Archived. Default for the system-prompt list.
    Default,
    All,
}

/// Write-side storage for skills.
pub struct SkillStore {
    root: PathBuf,
    lock_path: PathBuf,
    skill_max_chars: usize,
    supporting_file_max_bytes: u64,
    in_process_lock: std::sync::Mutex<()>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    version: String,
}

/// Graveyard entry written when a skill is deleted with `absorbed_into`.
#[derive(Debug, Serialize, Deserialize)]
struct GraveyardEntry {
    name: String,
    deleted_at: DateTime<Utc>,
    absorbed_into: String,
}

impl SkillStore {
    /// Open (or create) a store at `root`.
    pub fn open(
        root: impl Into<PathBuf>,
        skill_max_chars: usize,
        supporting_file_max_bytes: usize,
    ) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root: root.clone(),
            lock_path: root.join(".lock"),
            skill_max_chars,
            supporting_file_max_bytes: supporting_file_max_bytes as u64,
            in_process_lock: std::sync::Mutex::new(()),
        })
    }

    /// Create a new skill.
    ///
    /// `version` must be set; the `niles-capabilities` loader requires
    /// it in the frontmatter. Default to `"0.1.0"` if the caller has
    /// no other source for the value.
    pub fn create(
        &self,
        name: &str,
        description: &str,
        version: &str,
        body: &str,
        provenance: Provenance,
    ) -> Result<()> {
        validate_skill_name(name)?;
        scan::scan(name)?;
        scan::scan(description)?;
        scan::scan(version)?;
        scan::scan(body)?;

        let char_count = body.chars().count();
        if char_count > self.skill_max_chars {
            return Err(Error::TooLarge {
                reason: format!(
                    "body is {} chars (max {})",
                    char_count, self.skill_max_chars
                ),
            });
        }

        self.with_store_lock(|| {
            let dir = self.root.join(name);
            if dir.exists() {
                return Err(Error::AlreadyExists {
                    name: name.to_string(),
                });
            }
            std::fs::create_dir(&dir)?;

            let skill_md = format_skill_md(name, description, version, body);
            atomic_write(&dir.join("SKILL.md"), skill_md.as_bytes())?;

            let sidecar = Sidecar::new(provenance);
            sidecar.write(&dir.join(".usage.json"))?;

            Ok(())
        })
    }

    /// Load a skill by exact name.
    pub fn load(&self, name: &str) -> Result<Skill> {
        validate_skill_name(name)?;
        let dir = self.root.join(name);
        if !dir.exists() {
            return Err(Error::NotFound {
                name: name.to_string(),
            });
        }

        let skill_md_path = dir.join("SKILL.md");
        let md_meta = std::fs::metadata(&skill_md_path)?;
        // Guard against externally bloated files before reading into memory.
        let max_bytes = self.skill_max_chars as u64 * 4 + 4096;
        if md_meta.len() > max_bytes {
            return Err(Error::TooLarge {
                reason: format!(
                    "SKILL.md is {} bytes (max ~{} bytes for {} chars)",
                    md_meta.len(),
                    max_bytes,
                    self.skill_max_chars
                ),
            });
        }

        let raw = std::fs::read_to_string(skill_md_path)?;
        let (meta, body) = parse_skill_md(&raw, &dir)?;
        if meta.name != name {
            return Err(Error::ScanFailed {
                reason: format!(
                    "frontmatter name `{}` does not match skill directory `{}`",
                    meta.name, name
                ),
            });
        }
        let body_char_count = body.chars().count();
        if body_char_count > self.skill_max_chars {
            return Err(Error::TooLarge {
                reason: format!(
                    "body is {} chars (max {})",
                    body_char_count, self.skill_max_chars
                ),
            });
        }

        // Enforce supporting-file size limit.
        for entry in read_dir(&dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
            if file_name_str == "SKILL.md" || file_name_str == ".usage.json" {
                continue;
            }
            let meta_data = entry.metadata()?;
            if meta_data.len() > self.supporting_file_max_bytes {
                return Err(Error::TooLarge {
                    reason: format!(
                        "supporting file `{}` is {} bytes (max {})",
                        file_name_str,
                        meta_data.len(),
                        self.supporting_file_max_bytes
                    ),
                });
            }
        }

        let sidecar = Sidecar::read(&dir.join(".usage.json"))?;

        Ok(Skill {
            name: meta.name,
            description: meta.description,
            version: meta.version,
            body,
            sidecar,
        })
    }

    /// Patch an existing skill's body.
    pub fn patch(&self, name: &str, new_body: &str) -> Result<()> {
        validate_skill_name(name)?;
        scan::scan(new_body)?;

        let char_count = new_body.chars().count();
        if char_count > self.skill_max_chars {
            return Err(Error::TooLarge {
                reason: format!(
                    "body is {} chars (max {})",
                    char_count, self.skill_max_chars
                ),
            });
        }

        self.with_store_lock(|| {
            let dir = self.root.join(name);
            if !dir.exists() {
                return Err(Error::NotFound {
                    name: name.to_string(),
                });
            }

            let mut sidecar = Sidecar::read(&dir.join(".usage.json"))?;
            if sidecar.pinned && matches!(sidecar.provenance, Provenance::AgentCreated) {
                return Err(Error::Pinned {
                    name: name.to_string(),
                });
            }

            let raw = std::fs::read_to_string(dir.join("SKILL.md"))?;
            let (meta, _old_body) = parse_skill_md(&raw, &dir)?;

            let skill_md = format_skill_md(&meta.name, &meta.description, &meta.version, new_body);
            atomic_write(&dir.join("SKILL.md"), skill_md.as_bytes())?;

            sidecar.patch_count += 1;
            sidecar.last_patched_at = Some(Utc::now());
            sidecar.status = SkillStatus::Active;
            sidecar.write(&dir.join(".usage.json"))?;

            Ok(())
        })
    }

    /// Delete a skill. If `absorbed_into` is `Some`, writes a graveyard entry.
    pub fn delete(&self, name: &str, absorbed_into: Option<&str>) -> Result<()> {
        validate_skill_name(name)?;

        self.with_store_lock(|| {
            let dir = self.root.join(name);
            if !dir.exists() {
                return Err(Error::NotFound {
                    name: name.to_string(),
                });
            }

            let sidecar = Sidecar::read(&dir.join(".usage.json"))?;
            if sidecar.pinned {
                return Err(Error::Pinned {
                    name: name.to_string(),
                });
            }

            if let Some(target) = absorbed_into {
                let graveyard_dir = self.root.join(".absorbed");
                std::fs::create_dir_all(&graveyard_dir)?;
                let entry = GraveyardEntry {
                    name: name.to_string(),
                    deleted_at: Utc::now(),
                    absorbed_into: target.to_string(),
                };
                let raw = serde_json::to_vec_pretty(&entry)?;
                atomic_write(&graveyard_dir.join(format!("{}.json", name)), &raw)?;
            }

            std::fs::remove_dir_all(&dir)?;
            Ok(())
        })
    }

    /// List all skill names in alphabetical order.
    pub fn list(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let entries = match read_dir(&self.root) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == ".absorbed" || name_str == ".lock" {
                continue;
            }
            if !entry.metadata()?.is_dir() {
                continue;
            }
            if entry.path().join("SKILL.md").is_file() {
                names.push(name_str.into_owned());
            }
        }

        names.sort();
        Ok(names)
    }

    /// List skill summaries (metadata only, no body) in alphabetical order.
    /// Silently skips malformed skills rather than failing the whole list.
    /// Excludes Archived skills by default.
    pub fn list_summaries(&self) -> Result<Vec<SkillSummary>> {
        self.list_summaries_filtered(SkillStatusFilter::Default)
    }

    /// List skill summaries with a status filter.
    /// Silently skips malformed skills rather than failing the whole list.
    pub fn list_summaries_filtered(&self, filter: SkillStatusFilter) -> Result<Vec<SkillSummary>> {
        let mut summaries = Vec::new();
        let entries = match read_dir(&self.root) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let max_bytes = self.skill_max_chars as u64 * 4 + 4096;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "skipping unreadable directory entry in list_summaries");
                    continue;
                }
            };
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == ".absorbed" || name_str == ".lock" {
                continue;
            }
            if validate_skill_name(&name_str).is_err() {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(skill = %name_str, error = %e, "skipping unreadable metadata in list_summaries");
                    continue;
                }
            };
            if !meta.is_dir() {
                continue;
            }
            let skill_md_path = entry.path().join("SKILL.md");
            if !skill_md_path.is_file() {
                continue;
            }

            let raw = match std::fs::read_to_string(&skill_md_path) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(skill = %name_str, error = %e, "skipping malformed skill in list_summaries");
                    continue;
                }
            };

            if raw.len() as u64 > max_bytes {
                tracing::warn!(skill = %name_str, "skipping oversized skill in list_summaries");
                continue;
            }

            let (meta, _) = match parse_skill_md(&raw, &entry.path()) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(skill = %name_str, error = %e, "skipping malformed skill in list_summaries");
                    continue;
                }
            };

            if meta.name != name_str {
                tracing::warn!(
                    skill = %name_str,
                    frontmatter_name = %meta.name,
                    "skipping skill with mismatched frontmatter name in list_summaries"
                );
                continue;
            }

            let sidecar = match Sidecar::read(&entry.path().join(".usage.json")) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(skill = %name_str, error = %e, "skipping malformed skill in list_summaries");
                    continue;
                }
            };

            if filter == SkillStatusFilter::Default && sidecar.status == SkillStatus::Archived {
                continue;
            }

            let last_activity_at = sidecar.latest_activity_at();
            summaries.push(SkillSummary {
                name: meta.name,
                description: meta.description,
                version: meta.version,
                pinned: sidecar.pinned,
                provenance: sidecar.provenance,
                status: sidecar.status,
                last_activity_at,
            });
        }

        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(summaries)
    }

    /// Increment usage counter for a skill. Silently succeeds if the skill does not exist.
    pub fn bump_use(&self, name: &str) -> Result<()> {
        self.bump(name, |s| {
            s.usage_count += 1;
            s.last_used_at = Some(Utc::now());
        })
    }

    /// Increment view counter for a skill. Silently succeeds if the skill does not exist.
    pub fn bump_view(&self, name: &str) -> Result<()> {
        self.bump(name, |s| {
            s.view_count += 1;
            s.last_viewed_at = Some(Utc::now());
        })
    }

    fn bump<F>(&self, name: &str, mut update: F) -> Result<()>
    where
        F: FnMut(&mut Sidecar),
    {
        validate_skill_name(name)?;

        self.with_store_lock(|| {
            let dir = self.root.join(name);
            if !dir.exists() {
                return Ok(());
            }
            let sidecar_path = dir.join(".usage.json");
            let mut sidecar = match Sidecar::read(&sidecar_path) {
                Ok(s) => s,
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            update(&mut sidecar);
            sidecar.status = SkillStatus::Active;
            if let Err(e) = sidecar.write(&sidecar_path) {
                tracing::warn!(skill = name, error = %e, "failed to write sidecar");
            }
            Ok(())
        })
    }

    /// Set the pinned flag on a skill.
    pub fn set_pinned(&self, name: &str, pinned: bool) -> Result<()> {
        validate_skill_name(name)?;

        self.with_store_lock(|| {
            let dir = self.root.join(name);
            if !dir.exists() {
                return Err(Error::NotFound {
                    name: name.to_string(),
                });
            }
            let mut sidecar = Sidecar::read(&dir.join(".usage.json"))?;
            sidecar.pinned = pinned;
            sidecar.write(&dir.join(".usage.json"))?;
            Ok(())
        })
    }

    /// Set the lifecycle status on a skill.
    pub fn set_status(&self, name: &str, status: SkillStatus) -> Result<()> {
        validate_skill_name(name)?;
        self.with_store_lock(|| {
            let dir = self.root.join(name);
            if !dir.exists() {
                return Err(Error::NotFound {
                    name: name.to_string(),
                });
            }
            let mut sidecar = Sidecar::read(&dir.join(".usage.json"))?;
            sidecar.status = status;
            sidecar.write(&dir.join(".usage.json"))?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // Lock helper
    // ------------------------------------------------------------------

    fn with_store_lock<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T>,
    {
        let _process_guard = self.in_process_lock.lock().unwrap();

        let lock_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.lock_path)?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);

        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() >= timeout {
                        return Err(Error::Locked);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => return Err(e.into()),
            }
        }

        let result = f();
        // Advisory lock released when `lock_file` is dropped.
        result
    }
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn validate_skill_name(name: &str) -> Result<()> {
    if name == "." || name == ".." || name == ".absorbed" {
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

fn format_skill_md(name: &str, description: &str, version: &str, body: &str) -> String {
    let yaml = serde_yaml::to_string(&Frontmatter {
        name: name.to_string(),
        description: description.to_string(),
        version: version.to_string(),
    })
    .unwrap();
    let yaml = yaml.trim_end_matches('\n');
    format!("---\n{}\n---\n{}", yaml, body)
}

fn parse_skill_md(raw: &str, _dir: &Path) -> Result<(Frontmatter, String)> {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);

    let first_newline = raw.find('\n');
    let first_line = match first_newline {
        Some(idx) => raw[..idx].trim_end_matches('\r'),
        None => raw.trim_end_matches('\r'),
    };
    if first_line != "---" {
        return Err(Error::ScanFailed {
            reason: "missing opening frontmatter delimiter".into(),
        });
    }

    let Some(open_end) = first_newline else {
        return Err(Error::ScanFailed {
            reason: "missing closing frontmatter delimiter".into(),
        });
    };
    let after_open = &raw[open_end + 1..];

    let mut search = 0;
    let close = loop {
        let line_end = after_open[search..]
            .find('\n')
            .map(|i| search + i)
            .unwrap_or(after_open.len());
        let line = after_open[search..line_end].trim_end_matches('\r');
        if line == "---" {
            break Some((search, line_end));
        }
        if line_end >= after_open.len() {
            break None;
        }
        search = line_end + 1;
    };

    let Some((close_start, close_end)) = close else {
        return Err(Error::ScanFailed {
            reason: "missing closing frontmatter delimiter".into(),
        });
    };

    let yaml = &after_open[..close_start];
    let body_start = (close_end + 1).min(after_open.len());
    let body = after_open[body_start..].to_string();

    let meta: Frontmatter = serde_yaml::from_str(yaml).map_err(|e| Error::ScanFailed {
        reason: format!("invalid frontmatter: {e}"),
    })?;

    Ok((meta, body))
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn store(tmp: &TempDir) -> SkillStore {
        SkillStore::open(tmp.path(), 100_000, 1_048_576).unwrap()
    }

    #[test]
    fn open_creates_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("nested");
        assert!(!root.exists());
        let _store = SkillStore::open(&root, 100_000, 1_048_576).unwrap();
        assert!(root.exists());
    }

    #[test]
    fn create_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);

        store
            .create(
                "my-skill",
                "My skill",
                "0.1.0",
                "# Body\n\nHello.",
                Provenance::UserCreated,
            )
            .unwrap();
        let skill = store.load("my-skill").unwrap();
        assert_eq!(skill.name, "my-skill");
        assert_eq!(skill.description, "My skill");
        assert_eq!(skill.body, "# Body\n\nHello.");
    }

    #[test]
    fn create_twice_errors() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("dup", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        let err = store
            .create("dup", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap_err();
        assert!(matches!(err, Error::AlreadyExists { name } if name == "dup"));
    }

    #[test]
    fn invalid_names_rejected() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);

        for bad in ["Foo", "with spaces", "..", "kitchen.for.dinner"] {
            let err = store
                .create(bad, "D", "0.1.0", "B", Provenance::UserCreated)
                .unwrap_err();
            assert!(
                matches!(err, Error::InvalidName { ref name, .. } if name == bad),
                "expected InvalidName for {bad}, got {err:?}"
            );
        }
    }

    #[test]
    fn body_with_zwsp_rejected() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        let err = store
            .create(
                "ok",
                "D",
                "0.1.0",
                "bad\u{200B}body",
                Provenance::UserCreated,
            )
            .unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
    }

    #[test]
    fn body_over_limit_rejected() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::open(tmp.path(), 10, 1_048_576).unwrap();
        let err = store
            .create("ok", "D", "0.1.0", "12345678901", Provenance::UserCreated)
            .unwrap_err();
        assert!(matches!(err, Error::TooLarge { .. }));
    }

    #[test]
    fn supporting_file_over_limit_rejected_on_load() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::open(tmp.path(), 100_000, 10).unwrap();
        store
            .create("ok", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();

        // Write a supporting file that exceeds the limit.
        std::fs::write(store.root.join("ok").join("extra.txt"), "12345678901").unwrap();

        let err = store.load("ok").unwrap_err();
        assert!(matches!(err, Error::TooLarge { .. }));
    }

    #[test]
    fn patch_updates_body_preserves_meta() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "Desc", "0.1.0", "Old", Provenance::UserCreated)
            .unwrap();

        let created_at = store.load("s").unwrap().sidecar.created_at;

        store.patch("s", "New body").unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(skill.body, "New body");
        assert_eq!(skill.description, "Desc");
        assert_eq!(skill.sidecar.created_at, created_at);
        assert_eq!(skill.sidecar.patch_count, 1);
        assert!(skill.sidecar.last_patched_at.is_some());
    }

    #[test]
    fn patch_pinned_agent_created_errors() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store.set_pinned("s", true).unwrap();
        let err = store.patch("s", "New").unwrap_err();
        assert!(matches!(err, Error::Pinned { name } if name == "s"));
    }

    #[test]
    fn patch_pinned_user_created_succeeds() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_pinned("s", true).unwrap();
        store.patch("s", "New").unwrap();
        assert_eq!(store.load("s").unwrap().body, "New");
    }

    #[test]
    fn delete_removes_directory() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.delete("s", None).unwrap();
        assert!(!store.root.join("s").exists());
    }

    #[test]
    fn delete_writes_graveyard_when_absorbed() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.delete("s", Some("other")).unwrap();

        let graveyard = store.root.join(".absorbed").join("s.json");
        assert!(graveyard.exists());
        let entry: GraveyardEntry =
            serde_json::from_str(&std::fs::read_to_string(&graveyard).unwrap()).unwrap();
        assert_eq!(entry.name, "s");
        assert_eq!(entry.absorbed_into, "other");
    }

    #[test]
    fn delete_pinned_errors() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_pinned("s", true).unwrap();
        let err = store.delete("s", None).unwrap_err();
        assert!(matches!(err, Error::Pinned { name } if name == "s"));
    }

    #[test]
    fn bump_use_increments_counter() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.bump_use("s").unwrap();
        store.bump_use("s").unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(skill.sidecar.usage_count, 2);
        assert!(skill.sidecar.last_used_at.is_some());
    }

    #[test]
    fn bump_use_on_missing_is_silent() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store.bump_use("missing").unwrap();
    }

    #[test]
    fn bump_use_on_corrupt_sidecar_returns_error() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        std::fs::write(store.root.join("s").join(".usage.json"), "not json").unwrap();
        let err = store.bump_use("s").unwrap_err();
        assert!(
            matches!(err, Error::Json(..)),
            "expected JSON parse error, got {err:?}"
        );
    }

    #[test]
    fn set_pinned_then_delete_errors() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_pinned("s", true).unwrap();
        let err = store.delete("s", None).unwrap_err();
        assert!(matches!(err, Error::Pinned { .. }));
    }

    #[test]
    fn list_returns_sorted_names() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("zebra", "Z", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store
            .create("alpha", "A", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();

        let names = store.list().unwrap();
        assert_eq!(names, vec!["alpha", "zebra"]);
    }

    #[test]
    fn list_skips_non_skill_directories() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("skill", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        std::fs::create_dir(store.root.join("not-a-skill")).unwrap();

        let names = store.list().unwrap();
        assert_eq!(names, vec!["skill"]);
    }

    #[test]
    fn load_not_found() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        let err = store.load("nope").unwrap_err();
        assert!(matches!(err, Error::NotFound { name } if name == "nope"));
    }

    #[test]
    fn delete_not_found() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        let err = store.delete("nope", None).unwrap_err();
        assert!(matches!(err, Error::NotFound { name } if name == "nope"));
    }

    #[test]
    fn patch_not_found() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        let err = store.patch("nope", "body").unwrap_err();
        assert!(matches!(err, Error::NotFound { name } if name == "nope"));
    }

    #[test]
    fn set_pinned_not_found() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        let err = store.set_pinned("nope", true).unwrap_err();
        assert!(matches!(err, Error::NotFound { name } if name == "nope"));
    }

    #[test]
    fn load_rejects_name_mismatch_in_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("skill-a", "Desc", "0.1.0", "Body", Provenance::UserCreated)
            .unwrap();
        let tampered = format_skill_md("different-name", "Desc", "0.1.0", "Body");
        std::fs::write(store.root.join("skill-a").join("SKILL.md"), tampered).unwrap();

        let err = store.load("skill-a").unwrap_err();
        assert!(matches!(err, Error::ScanFailed { .. }));
    }

    #[test]
    fn load_rejects_body_over_char_limit() {
        let tmp = TempDir::new().unwrap();
        let store = SkillStore::open(tmp.path(), 10, 1_048_576).unwrap();
        store
            .create(
                "skill",
                "Desc",
                "0.1.0",
                "1234567890",
                Provenance::UserCreated,
            )
            .unwrap();
        let oversized = format_skill_md("skill", "Desc", "0.1.0", "12345678901");
        std::fs::write(store.root.join("skill").join("SKILL.md"), oversized).unwrap();

        let err = store.load("skill").unwrap_err();
        assert!(matches!(err, Error::TooLarge { .. }));
    }

    #[test]
    fn concurrent_same_skill_create_is_atomic() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(store(&tmp));
        let mut handles = Vec::new();

        for _ in 0..10 {
            let s = store.clone();
            handles.push(std::thread::spawn(move || {
                s.create("contended", "D", "0.1.0", "B", Provenance::UserCreated)
            }));
        }

        let mut successes = 0;
        let mut already_exists = 0;
        for h in handles {
            match h.join().unwrap() {
                Ok(()) => successes += 1,
                Err(Error::AlreadyExists { .. }) => already_exists += 1,
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }

        assert_eq!(successes, 1, "exactly one thread should succeed");
        assert_eq!(
            already_exists, 9,
            "remaining threads should see AlreadyExists"
        );
        assert_eq!(store.list().unwrap(), vec!["contended"]);
    }

    /// Regression: a skill minted by `SkillStore::create` MUST be loadable
    /// by `niles_capabilities::load_capabilities` — they share the
    /// agentskills.io on-disk format and PR 2 of Phase 8 will load
    /// minted skills via the same loader.
    #[test]
    fn minted_skill_round_trips_through_capabilities_loader() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create(
                "my-skill",
                "What it does",
                "0.1.0",
                "# Body\n\nSomething useful.",
                Provenance::AgentCreated,
            )
            .unwrap();

        let loader = niles_capabilities::CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let cap = loader
            .get("my-skill")
            .expect("minted skill must be visible to niles-capabilities");
        assert_eq!(cap.metadata.description, "What it does");
        assert_eq!(cap.metadata.version, "0.1.0");
        assert!(cap.body.contains("Something useful."));
    }

    #[test]
    fn list_summaries_returns_metadata_per_skill() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create(
                "zebra",
                "Z desc",
                "0.2.0",
                "Z body",
                Provenance::UserCreated,
            )
            .unwrap();
        store
            .create(
                "alpha",
                "A desc",
                "0.1.0",
                "A body",
                Provenance::UserCreated,
            )
            .unwrap();

        let summaries = store.list_summaries().unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].name, "alpha");
        assert_eq!(summaries[0].description, "A desc");
        assert_eq!(summaries[0].version, "0.1.0");
        assert_eq!(summaries[0].status, SkillStatus::Active);
        assert!(summaries[0].last_activity_at.is_none());
        assert_eq!(summaries[1].name, "zebra");
        assert_eq!(summaries[1].description, "Z desc");
        assert_eq!(summaries[1].version, "0.2.0");
        assert_eq!(summaries[1].status, SkillStatus::Active);
        assert!(summaries[1].last_activity_at.is_none());
    }

    #[test]
    fn list_summaries_includes_pinned_and_provenance() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("user-skill", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store
            .create("agent-skill", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store.set_pinned("user-skill", true).unwrap();

        let summaries = store.list_summaries().unwrap();
        assert_eq!(summaries.len(), 2);

        let user = summaries.iter().find(|s| s.name == "user-skill").unwrap();
        assert!(user.pinned);
        assert_eq!(user.provenance, Provenance::UserCreated);
        assert_eq!(user.status, SkillStatus::Active);
        assert!(user.last_activity_at.is_none());

        let agent = summaries.iter().find(|s| s.name == "agent-skill").unwrap();
        assert!(!agent.pinned);
        assert_eq!(agent.provenance, Provenance::AgentCreated);
        assert_eq!(agent.status, SkillStatus::Active);
        assert!(agent.last_activity_at.is_none());
    }

    #[test]
    fn list_summaries_skips_malformed_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("good", "Good desc", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store
            .create("bad", "Bad desc", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        std::fs::write(store.root.join("bad").join("SKILL.md"), "not yaml at all").unwrap();

        let summaries = store.list_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "good");
    }

    #[test]
    fn list_summaries_skips_name_mismatch() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("good", "Good desc", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store
            .create("bad", "Bad desc", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        let tampered = format_skill_md("different-name", "Bad desc", "0.1.0", "B");
        std::fs::write(store.root.join("bad").join("SKILL.md"), tampered).unwrap();

        let summaries = store.list_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "good");
    }

    #[test]
    fn list_summaries_skips_invalid_name() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("good", "Good desc", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();

        // Simulate a hand-created directory with an invalid name.
        let hidden = store.root.join(".hidden");
        std::fs::create_dir(&hidden).unwrap();
        std::fs::write(
            hidden.join("SKILL.md"),
            "---\nname: .hidden\ndescription: H\nversion: 0.1.0\n---\nB",
        )
        .unwrap();
        std::fs::write(
            hidden.join(".usage.json"),
            r#"{"created_at":"2026-01-01T12:00:00Z","provenance":"user-created"}"#,
        )
        .unwrap();

        let summaries = store.list_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "good");
    }

    #[test]
    fn list_summaries_skips_missing_sidecar() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("skill", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        std::fs::remove_file(store.root.join("skill").join(".usage.json")).unwrap();

        let summaries = store.list_summaries().unwrap();
        assert!(summaries.is_empty());
    }

    #[test]
    fn list_summaries_empty_when_no_skills() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        let summaries = store.list_summaries().unwrap();
        assert!(summaries.is_empty());
    }

    #[test]
    fn bump_view_increments_counter_and_timestamp() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.bump_view("s").unwrap();
        store.bump_view("s").unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(skill.sidecar.view_count, 2);
        assert!(skill.sidecar.last_viewed_at.is_some());
    }

    #[test]
    fn bump_view_on_missing_is_silent() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store.bump_view("missing").unwrap();
    }

    #[test]
    fn bump_view_then_latest_activity_returns_view_ts() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.bump_view("s").unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(
            skill.sidecar.latest_activity_at(),
            skill.sidecar.last_viewed_at
        );
    }

    #[test]
    fn sidecar_back_compat_old_format() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        let old = r#"{"created_at":"2026-01-01T12:00:00Z","provenance":"user-created"}"#;
        std::fs::write(store.root.join("s").join(".usage.json"), old).unwrap();

        let skill = store.load("s").unwrap();
        assert_eq!(skill.sidecar.view_count, 0);
        assert!(skill.sidecar.last_viewed_at.is_none());
    }

    #[test]
    fn list_summaries_default_excludes_archived() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("active-skill", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store
            .create("archived-skill", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store
            .set_status("archived-skill", SkillStatus::Archived)
            .unwrap();

        let summaries = store.list_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "active-skill");
    }

    #[test]
    fn list_summaries_filtered_all_includes_archived() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("active-skill", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store
            .create("archived-skill", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store
            .set_status("archived-skill", SkillStatus::Archived)
            .unwrap();

        let summaries = store
            .list_summaries_filtered(SkillStatusFilter::All)
            .unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn set_status_writes_to_disk() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_status("s", SkillStatus::Stale).unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(skill.sidecar.status, SkillStatus::Stale);
    }

    #[test]
    fn set_status_not_found_errors() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        let err = store.set_status("nope", SkillStatus::Archived).unwrap_err();
        assert!(matches!(err, Error::NotFound { name } if name == "nope"));
    }

    #[test]
    fn bump_use_revives_stale_skill() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_status("s", SkillStatus::Stale).unwrap();
        store.bump_use("s").unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(skill.sidecar.status, SkillStatus::Active);
    }

    #[test]
    fn bump_view_revives_archived_skill() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_status("s", SkillStatus::Archived).unwrap();
        store.bump_view("s").unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(skill.sidecar.status, SkillStatus::Active);
    }

    #[test]
    fn patch_revives_stale_skill() {
        let tmp = TempDir::new().unwrap();
        let store = store(&tmp);
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_status("s", SkillStatus::Stale).unwrap();
        store.patch("s", "new body").unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(skill.sidecar.status, SkillStatus::Active);
    }
}
