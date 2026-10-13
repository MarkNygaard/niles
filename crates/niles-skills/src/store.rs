//! SkillStore — the skills Niles has made, and the household asked for.
//!
//! They live here, in memory, and every change is handed whole to a
//! [`SkillPersistence`] — in Niles, a Postgres row. They used to be
//! directories under `[skills].directory`, which has no default, so a
//! Niles without that section could not learn anything at all.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::scan;
use crate::sidecar::{Provenance, Sidecar, SkillStatus};

/// A loaded skill with metadata, body, and telemetry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Where the changed set goes to be kept. Called with the lock held, so
/// documents arrive in the order they were written; it should only hand
/// the document on, not wait for it to be stored.
pub trait SkillPersistence: Send + Sync {
    fn store(&self, document: &str);
}

/// Where a skill went when it was folded into another.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Absorbed {
    name: String,
    deleted_at: DateTime<Utc>,
    absorbed_into: String,
}

/// The longest description a new skill may have. It is all a model sees
/// of a skill when deciding whether to open it, so it has to say when
/// to use it in a line: "Use when the house is wound down at night."
pub const MAX_DESCRIPTION_CHARS: usize = 60;

/// How many changes are kept to undo, across all skills. Skills are
/// written by Niles while nobody watches; this is how somebody puts one
/// back the way it was.
const MAX_REVISIONS: usize = 100;

/// One change to one skill: what it was before and after. `None` before
/// is a skill made; `None` after is one deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Revision {
    at: DateTime<Utc>,
    name: String,
    change: String,
    before: Option<Skill>,
    after: Option<Skill>,
}

/// A change as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub at: DateTime<Utc>,
    /// "made", "edited", "rewritten", "folded into evening", "deleted".
    pub change: String,
}

/// When the set was last tidied, and what the tidy-up said it did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tidied {
    pub at: DateTime<Utc>,
    pub summary: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Document {
    #[serde(default)]
    skills: Vec<Skill>,
    #[serde(default)]
    absorbed: Vec<Absorbed>,
    #[serde(default)]
    revisions: Vec<Revision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tidied: Option<Tidied>,
}

#[derive(Default)]
struct Skills {
    by_name: BTreeMap<String, Skill>,
    absorbed: Vec<Absorbed>,
    revisions: Vec<Revision>,
    tidied: Option<Tidied>,
}

impl Skills {
    fn revise(&mut self, name: &str, change: &str, before: Option<Skill>) {
        let after = self.by_name.get(name).cloned();
        self.revisions.push(Revision {
            at: Utc::now(),
            name: name.to_string(),
            change: change.to_string(),
            before,
            after,
        });
        if self.revisions.len() > MAX_REVISIONS {
            let excess = self.revisions.len() - MAX_REVISIONS;
            self.revisions.drain(..excess);
        }
    }
}

/// The household's skills.
pub struct SkillStore {
    skill_max_chars: usize,
    skills: Mutex<Skills>,
    sink: Option<Arc<dyn SkillPersistence>>,
}

impl SkillStore {
    /// Empty, kept only as long as the process.
    pub fn new(skill_max_chars: usize) -> Self {
        Self {
            skill_max_chars,
            skills: Mutex::new(Skills::default()),
            sink: None,
        }
    }

    /// Start from a set kept earlier. Refused whole when any skill in it
    /// fails the checks a new one would: skills go into every prompt,
    /// and quietly dropping one would delete it at the next save.
    pub fn from_json(document: &str, skill_max_chars: usize) -> Result<Self> {
        let document: Document = serde_json::from_str(document)?;
        let store = Self::new(skill_max_chars);
        {
            let mut skills = store.lock();
            for skill in document.skills {
                store.check(&skill.name, &skill.description, &skill.version, &skill.body)?;
                skills.by_name.insert(skill.name.clone(), skill);
            }
            skills.absorbed = document.absorbed;
            skills.revisions = document.revisions;
            skills.tidied = document.tidied;
        }
        Ok(store)
    }

    /// Hand every change to `sink`.
    pub fn with_sink(mut self, sink: Arc<dyn SkillPersistence>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn lock(&self) -> MutexGuard<'_, Skills> {
        self.skills.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check(&self, name: &str, description: &str, version: &str, body: &str) -> Result<()> {
        validate_skill_name(name)?;
        scan::scan(name)?;
        scan::scan(description)?;
        scan::scan(version)?;
        self.check_body(body)
    }

    fn check_body(&self, body: &str) -> Result<()> {
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
        Ok(())
    }

    /// Run `f` on the set and, when it changed something, hand the set on.
    fn change<T>(&self, f: impl FnOnce(&mut Skills) -> Result<(T, bool)>) -> Result<T> {
        let mut skills = self.lock();
        let (out, changed) = f(&mut skills)?;
        if changed && let Some(sink) = &self.sink {
            let document = Document {
                skills: skills.by_name.values().cloned().collect(),
                absorbed: skills.absorbed.clone(),
                revisions: skills.revisions.clone(),
                tidied: skills.tidied.clone(),
            };
            match serde_json::to_string(&document) {
                Ok(document) => sink.store(&document),
                Err(e) => tracing::error!(error = %e, "skills could not be written down"),
            }
        }
        Ok(out)
    }

    fn named<'a>(skills: &'a mut Skills, name: &str) -> Result<&'a mut Skill> {
        skills.by_name.get_mut(name).ok_or_else(|| Error::NotFound {
            name: name.to_string(),
        })
    }

    /// Create a new skill.
    ///
    /// `version` must be set. Default to `"0.1.0"` if the caller has no
    /// other source for the value.
    pub fn create(
        &self,
        name: &str,
        description: &str,
        version: &str,
        body: &str,
        provenance: Provenance,
    ) -> Result<()> {
        self.check(name, description, version, body)?;
        let length = description.chars().count();
        if length > MAX_DESCRIPTION_CHARS {
            return Err(Error::TooLarge {
                reason: format!(
                    "the description is {length} characters (max {MAX_DESCRIPTION_CHARS}); \
                     it is all a model sees when choosing, so say when to use it: \
                     \"Use when ...\""
                ),
            });
        }
        self.change(|skills| {
            if skills.by_name.contains_key(name) {
                return Err(Error::AlreadyExists {
                    name: name.to_string(),
                });
            }
            skills.by_name.insert(
                name.to_string(),
                Skill {
                    name: name.to_string(),
                    description: description.to_string(),
                    version: version.to_string(),
                    body: body.to_string(),
                    sidecar: Sidecar::new(provenance),
                },
            );
            skills.absorbed.retain(|a| a.name != name);
            skills.revise(name, "made", None);
            Ok(((), true))
        })
    }

    /// Load a skill by exact name. A skill folded into another says
    /// where it went.
    pub fn load(&self, name: &str) -> Result<Skill> {
        validate_skill_name(name)?;
        let skills = self.lock();
        if let Some(skill) = skills.by_name.get(name) {
            return Ok(skill.clone());
        }
        match skills.absorbed.iter().find(|a| a.name == name) {
            Some(a) => Err(Error::Absorbed {
                name: name.to_string(),
                into: a.absorbed_into.clone(),
            }),
            None => Err(Error::NotFound {
                name: name.to_string(),
            }),
        }
    }

    /// Replace an existing skill's whole body. For a small change,
    /// [`Self::edit`]: a body rewritten from memory loses what the
    /// writer did not remember.
    pub fn patch(&self, name: &str, new_body: &str) -> Result<()> {
        validate_skill_name(name)?;
        self.check_body(new_body)?;
        self.rewrite(name, "rewritten", |_| Ok(new_body.to_string()))
    }

    /// Replace the one place `old` appears in a skill's body with `new`.
    pub fn edit(&self, name: &str, old: &str, new: &str) -> Result<()> {
        validate_skill_name(name)?;
        if old.is_empty() {
            return Err(Error::EditNotFound {
                name: name.to_string(),
            });
        }
        self.rewrite(name, "edited", |body| match body.matches(old).count() {
            0 => Err(Error::EditNotFound {
                name: name.to_string(),
            }),
            1 => Ok(body.replacen(old, new, 1)),
            count => Err(Error::EditAmbiguous {
                name: name.to_string(),
                count,
            }),
        })
    }

    fn rewrite(
        &self,
        name: &str,
        change: &str,
        body: impl FnOnce(&str) -> Result<String>,
    ) -> Result<()> {
        self.change(|skills| {
            let skill = Self::named(skills, name)?;
            if skill.sidecar.pinned && matches!(skill.sidecar.provenance, Provenance::AgentCreated)
            {
                return Err(Error::Pinned {
                    name: name.to_string(),
                });
            }
            let new_body = body(&skill.body)?;
            self.check_body(&new_body)?;
            let before = skill.clone();
            skill.body = new_body;
            skill.sidecar.patch_count += 1;
            skill.sidecar.last_patched_at = Some(Utc::now());
            skill.sidecar.status = SkillStatus::Active;
            skills.revise(name, change, Some(before));
            Ok(((), true))
        })
    }

    /// Put a skill away because another now holds what it said. Archived,
    /// not deleted, and `into` must exist: a skill retired without
    /// anywhere to go is just lost. What Niles does on its own, where a
    /// person would delete.
    pub fn retire(&self, name: &str, into: &str) -> Result<()> {
        validate_skill_name(name)?;
        validate_skill_name(into)?;
        self.change(|skills| {
            if name == into || !skills.by_name.contains_key(into) {
                return Err(Error::NotFound {
                    name: into.to_string(),
                });
            }
            let skill = Self::named(skills, name)?;
            if skill.sidecar.pinned {
                return Err(Error::Pinned {
                    name: name.to_string(),
                });
            }
            let before = skill.clone();
            skill.sidecar.status = SkillStatus::Archived;
            skill.sidecar.absorbed_into = Some(into.to_string());
            skills.revise(name, &format!("folded into {into}"), Some(before));
            Ok(((), true))
        })
    }

    /// Put the skill back as it was before its last change, and forget
    /// that change: undoing again goes one further back.
    pub fn undo(&self, name: &str) -> Result<Change> {
        validate_skill_name(name)?;
        self.change(|skills| {
            let at = skills
                .revisions
                .iter()
                .rposition(|r| r.name == name)
                .ok_or_else(|| Error::NothingToUndo {
                    name: name.to_string(),
                })?;
            let revision = skills.revisions.remove(at);
            match revision.before {
                Some(skill) => {
                    skills.absorbed.retain(|a| a.name != name);
                    skills.by_name.insert(name.to_string(), skill);
                }
                None => {
                    skills.by_name.remove(name);
                }
            }
            Ok((
                Change {
                    at: revision.at,
                    change: revision.change,
                },
                true,
            ))
        })
    }

    /// The changes that can be undone for `name`, newest first.
    pub fn changes(&self, name: &str) -> Vec<Change> {
        self.lock()
            .revisions
            .iter()
            .rev()
            .filter(|r| r.name == name)
            .map(|r| Change {
                at: r.at,
                change: r.change.clone(),
            })
            .collect()
    }

    /// When the set was last tidied.
    pub fn tidied(&self) -> Option<Tidied> {
        self.lock().tidied.clone()
    }

    /// Note that the set was tidied, and what was done.
    pub fn mark_tidied(&self, summary: &str) {
        let _ = self.change(|skills| {
            skills.tidied = Some(Tidied {
                at: Utc::now(),
                summary: summary.to_string(),
            });
            Ok(((), true))
        });
    }

    /// Delete a skill. With `absorbed_into`, remember where it went.
    pub fn delete(&self, name: &str, absorbed_into: Option<&str>) -> Result<()> {
        validate_skill_name(name)?;
        self.change(|skills| {
            let before = Self::named(skills, name)?.clone();
            if before.sidecar.pinned {
                return Err(Error::Pinned {
                    name: name.to_string(),
                });
            }
            skills.by_name.remove(name);
            skills.revise(name, "deleted", Some(before));
            if let Some(target) = absorbed_into {
                skills.absorbed.retain(|a| a.name != name);
                skills.absorbed.push(Absorbed {
                    name: name.to_string(),
                    deleted_at: Utc::now(),
                    absorbed_into: target.to_string(),
                });
            }
            Ok(((), true))
        })
    }

    /// All skill names in alphabetical order.
    pub fn list(&self) -> Vec<String> {
        self.lock().by_name.keys().cloned().collect()
    }

    /// Every skill, body and all, in alphabetical order.
    pub fn all(&self) -> Vec<Skill> {
        self.lock().by_name.values().cloned().collect()
    }

    /// Skill summaries (metadata only, no body) in alphabetical order.
    /// Excludes Archived skills.
    pub fn list_summaries(&self) -> Vec<SkillSummary> {
        self.list_summaries_filtered(SkillStatusFilter::Default)
    }

    /// Skill summaries with a status filter.
    pub fn list_summaries_filtered(&self, filter: SkillStatusFilter) -> Vec<SkillSummary> {
        self.lock()
            .by_name
            .values()
            .filter(|s| {
                filter == SkillStatusFilter::All || s.sidecar.status != SkillStatus::Archived
            })
            .map(|s| SkillSummary {
                name: s.name.clone(),
                description: s.description.clone(),
                version: s.version.clone(),
                pinned: s.sidecar.pinned,
                provenance: s.sidecar.provenance.clone(),
                status: s.sidecar.status,
                last_activity_at: s.sidecar.latest_activity_at(),
            })
            .collect()
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

    fn bump(&self, name: &str, update: impl FnOnce(&mut Sidecar)) -> Result<()> {
        validate_skill_name(name)?;
        self.change(|skills| {
            let Some(skill) = skills.by_name.get_mut(name) else {
                return Ok(((), false));
            };
            update(&mut skill.sidecar);
            skill.sidecar.status = SkillStatus::Active;
            Ok(((), true))
        })
    }

    /// Set the pinned flag on a skill.
    pub fn set_pinned(&self, name: &str, pinned: bool) -> Result<()> {
        validate_skill_name(name)?;
        self.change(|skills| {
            Self::named(skills, name)?.sidecar.pinned = pinned;
            Ok(((), true))
        })
    }

    /// Set the lifecycle status on a skill.
    pub fn set_status(&self, name: &str, status: SkillStatus) -> Result<()> {
        validate_skill_name(name)?;
        self.change(|skills| {
            Self::named(skills, name)?.sidecar.status = status;
            Ok(((), true))
        })
    }

    /// Run the closure on the skill's telemetry under the lock, and
    /// write back only when it returns a different status.
    /// Returns `Some(new_status)` if a write occurred, `None` otherwise.
    pub fn update_status<F>(&self, name: &str, f: F) -> Result<Option<SkillStatus>>
    where
        F: FnOnce(&Sidecar) -> Option<SkillStatus>,
    {
        validate_skill_name(name)?;
        self.change(|skills| {
            let skill = Self::named(skills, name)?;
            match f(&skill.sidecar) {
                Some(new_status) if skill.sidecar.status != new_status => {
                    skill.sidecar.status = new_status;
                    Ok((Some(new_status), true))
                }
                _ => Ok((None, false)),
            }
        })
    }

    /// Replace a skill's telemetry, for tests that need it aged.
    #[cfg(test)]
    pub(crate) fn put_sidecar(&self, name: &str, sidecar: Sidecar) {
        self.lock().by_name.get_mut(name).unwrap().sidecar = sidecar;
    }
}

fn validate_skill_name(name: &str) -> Result<()> {
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

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SkillStore {
        SkillStore::new(100_000)
    }

    #[derive(Default)]
    struct Kept(Mutex<Vec<String>>);

    impl SkillPersistence for Kept {
        fn store(&self, document: &str) {
            self.0.lock().unwrap().push(document.to_string());
        }
    }

    impl Kept {
        fn count(&self) -> usize {
            self.0.lock().unwrap().len()
        }

        fn last(&self) -> String {
            self.0.lock().unwrap().last().cloned().unwrap()
        }
    }

    #[test]
    fn create_load_round_trip() {
        let store = store();
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
        assert_eq!(skill.version, "0.1.0");
        assert_eq!(skill.body, "# Body\n\nHello.");
        assert_eq!(skill.sidecar.provenance, Provenance::UserCreated);
    }

    #[test]
    fn create_twice_errors() {
        let store = store();
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
        let store = store();
        for bad in ["Foo", "with spaces", "..", "kitchen.for.dinner", ""] {
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
        let err = store()
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
        let store = SkillStore::new(10);
        let err = store
            .create(
                "s",
                "D",
                "0.1.0",
                "this body is too long",
                Provenance::UserCreated,
            )
            .unwrap_err();
        assert!(matches!(err, Error::TooLarge { .. }));
    }

    #[test]
    fn patch_updates_body_preserves_meta() {
        let store = store();
        store
            .create("s", "Desc", "1.2.3", "old", Provenance::AgentCreated)
            .unwrap();
        store.patch("s", "new").unwrap();
        let skill = store.load("s").unwrap();
        assert_eq!(skill.body, "new");
        assert_eq!(skill.description, "Desc");
        assert_eq!(skill.version, "1.2.3");
        assert_eq!(skill.sidecar.patch_count, 1);
        assert!(skill.sidecar.last_patched_at.is_some());
    }

    #[test]
    fn patch_pinned_agent_created_errors() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store.set_pinned("s", true).unwrap();
        let err = store.patch("s", "new").unwrap_err();
        assert!(matches!(err, Error::Pinned { .. }));
    }

    #[test]
    fn patch_pinned_user_created_succeeds() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_pinned("s", true).unwrap();
        store.patch("s", "new").unwrap();
        assert_eq!(store.load("s").unwrap().body, "new");
    }

    #[test]
    fn delete_removes_skill() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.delete("s", None).unwrap();
        assert!(matches!(
            store.load("s").unwrap_err(),
            Error::NotFound { .. }
        ));
        assert!(store.list().is_empty());
    }

    #[test]
    fn a_skill_folded_into_another_says_where_it_went() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store.delete("s", Some("other")).unwrap();
        let err = store.load("s").unwrap_err();
        assert!(
            matches!(err, Error::Absorbed { ref name, ref into } if name == "s" && into == "other"),
            "{err:?}"
        );
    }

    #[test]
    fn delete_pinned_errors() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_pinned("s", true).unwrap();
        let err = store.delete("s", None).unwrap_err();
        assert!(matches!(err, Error::Pinned { .. }));
        assert!(store.load("s").is_ok());
    }

    #[test]
    fn bump_use_increments_counter() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.bump_use("s").unwrap();
        store.bump_use("s").unwrap();
        let sidecar = store.load("s").unwrap().sidecar;
        assert_eq!(sidecar.usage_count, 2);
        assert!(sidecar.last_used_at.is_some());
    }

    #[test]
    fn bump_on_missing_is_silent() {
        let store = store();
        store.bump_use("nope").unwrap();
        store.bump_view("nope").unwrap();
    }

    #[test]
    fn bump_view_increments_counter_and_timestamp() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.bump_view("s").unwrap();
        let sidecar = store.load("s").unwrap().sidecar;
        assert_eq!(sidecar.view_count, 1);
        assert_eq!(sidecar.latest_activity_at(), sidecar.last_viewed_at);
    }

    #[test]
    fn list_returns_sorted_names() {
        let store = store();
        for name in ["zeta", "alpha", "mid"] {
            store
                .create(name, "D", "0.1.0", "B", Provenance::UserCreated)
                .unwrap();
        }
        assert_eq!(store.list(), vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn missing_skills_are_not_found() {
        let store = store();
        assert!(matches!(
            store.load("x").unwrap_err(),
            Error::NotFound { .. }
        ));
        assert!(matches!(
            store.patch("x", "B").unwrap_err(),
            Error::NotFound { .. }
        ));
        assert!(matches!(
            store.delete("x", None).unwrap_err(),
            Error::NotFound { .. }
        ));
        assert!(matches!(
            store.set_pinned("x", true).unwrap_err(),
            Error::NotFound { .. }
        ));
        assert!(matches!(
            store.set_status("x", SkillStatus::Stale).unwrap_err(),
            Error::NotFound { .. }
        ));
        assert!(matches!(
            store.update_status("x", |_| None).unwrap_err(),
            Error::NotFound { .. }
        ));
    }

    #[test]
    fn concurrent_same_skill_create_is_atomic() {
        let store = Arc::new(store());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let s = store.clone();
                std::thread::spawn(move || {
                    s.create("same", "D", "0.1.0", "B", Provenance::UserCreated)
                        .is_ok()
                })
            })
            .collect();
        let created = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(created, 1);
    }

    #[test]
    fn list_summaries_returns_metadata_per_skill() {
        let store = store();
        store
            .create("b", "Bee", "0.2.0", "B", Provenance::AgentCreated)
            .unwrap();
        store
            .create("a", "Ay", "0.1.0", "B", Provenance::UserCreated)
            .unwrap();
        store.set_pinned("b", true).unwrap();
        let summaries = store.list_summaries();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].name, "a");
        assert_eq!(summaries[0].description, "Ay");
        assert_eq!(summaries[0].provenance, Provenance::UserCreated);
        assert!(!summaries[0].pinned);
        assert_eq!(summaries[1].version, "0.2.0");
        assert_eq!(summaries[1].provenance, Provenance::AgentCreated);
        assert!(summaries[1].pinned);
    }

    #[test]
    fn list_summaries_empty_when_no_skills() {
        assert!(store().list_summaries().is_empty());
    }

    #[test]
    fn list_summaries_default_excludes_archived() {
        let store = store();
        store
            .create("live", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store
            .create("old", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store.set_status("old", SkillStatus::Archived).unwrap();
        let names: Vec<_> = store.list_summaries().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["live"]);
        let all: Vec<_> = store
            .list_summaries_filtered(SkillStatusFilter::All)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(all, vec!["live", "old"]);
    }

    #[test]
    fn use_view_and_patch_revive_a_skill() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store.set_status("s", SkillStatus::Stale).unwrap();
        store.bump_use("s").unwrap();
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Active);
        store.set_status("s", SkillStatus::Archived).unwrap();
        store.bump_view("s").unwrap();
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Active);
        store.set_status("s", SkillStatus::Stale).unwrap();
        store.patch("s", "new").unwrap();
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Active);
    }

    #[test]
    fn update_status_changes_and_returns_new() {
        let store = store();
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        let out = store
            .update_status("s", |_| Some(SkillStatus::Stale))
            .unwrap();
        assert_eq!(out, Some(SkillStatus::Stale));
        assert_eq!(store.load("s").unwrap().sidecar.status, SkillStatus::Stale);
        assert_eq!(store.update_status("s", |_| None).unwrap(), None);
        assert_eq!(
            store
                .update_status("s", |_| Some(SkillStatus::Stale))
                .unwrap(),
            None
        );
    }

    fn evening(store: &SkillStore) {
        store
            .create(
                "evening",
                "Use when winding the house down at night",
                "0.1.0",
                "1. Dim the living room to 30%.\n2. Turn off the kitchen.",
                Provenance::AgentCreated,
            )
            .unwrap();
    }

    #[test]
    fn a_new_skill_says_when_to_use_it_in_a_line() {
        let err = store()
            .create(
                "s",
                "A long description of everything this skill does in the evening",
                "0.1.0",
                "B",
                Provenance::AgentCreated,
            )
            .unwrap_err();
        assert!(
            matches!(err, Error::TooLarge { ref reason } if reason.contains("Use when")),
            "{err}"
        );
    }

    #[test]
    fn an_edit_replaces_one_place_and_keeps_the_rest() {
        let store = store();
        evening(&store);
        store.edit("evening", "30%", "20%").unwrap();
        assert_eq!(
            store.load("evening").unwrap().body,
            "1. Dim the living room to 20%.\n2. Turn off the kitchen."
        );
        assert!(matches!(
            store.edit("evening", "the bedroom", "x").unwrap_err(),
            Error::EditNotFound { .. }
        ));
        assert!(matches!(
            store.edit("evening", "the ", "x").unwrap_err(),
            Error::EditAmbiguous { count: 2, .. }
        ));
    }

    #[test]
    fn retiring_archives_into_a_skill_that_exists() {
        let store = store();
        evening(&store);
        store
            .create(
                "night",
                "Use when going to bed",
                "0.1.0",
                "Turn everything off.",
                Provenance::AgentCreated,
            )
            .unwrap();
        assert!(matches!(
            store.retire("night", "nowhere").unwrap_err(),
            Error::NotFound { .. }
        ));
        store.retire("night", "evening").unwrap();
        let night = store.load("night").unwrap();
        assert_eq!(night.sidecar.status, SkillStatus::Archived);
        assert_eq!(night.sidecar.absorbed_into.as_deref(), Some("evening"));
        store.set_pinned("evening", true).unwrap();
        assert!(matches!(
            store.retire("evening", "night").unwrap_err(),
            Error::Pinned { .. }
        ));
    }

    #[test]
    fn undo_walks_back_one_change_at_a_time() {
        let store = store();
        evening(&store);
        store.edit("evening", "30%", "20%").unwrap();
        store.patch("evening", "Turn everything off.").unwrap();
        let changes: Vec<_> = store
            .changes("evening")
            .into_iter()
            .map(|c| c.change)
            .collect();
        assert_eq!(changes, vec!["rewritten", "edited", "made"]);

        assert_eq!(store.undo("evening").unwrap().change, "rewritten");
        assert!(store.load("evening").unwrap().body.contains("20%"));
        assert_eq!(store.undo("evening").unwrap().change, "edited");
        assert!(store.load("evening").unwrap().body.contains("30%"));
        assert_eq!(store.undo("evening").unwrap().change, "made");
        assert!(store.load("evening").is_err());
        assert!(matches!(
            store.undo("evening").unwrap_err(),
            Error::NothingToUndo { .. }
        ));
    }

    #[test]
    fn a_deleted_skill_comes_back_with_undo() {
        let store = store();
        evening(&store);
        store.delete("evening", None).unwrap();
        store.undo("evening").unwrap();
        assert_eq!(store.load("evening").unwrap().name, "evening");
    }

    #[test]
    fn changes_and_the_tidy_up_outlast_a_restart() {
        let kept = Arc::new(Kept::default());
        let store = store().with_sink(kept.clone());
        evening(&store);
        store.edit("evening", "30%", "20%").unwrap();
        store.mark_tidied("Nothing needed tidying.");

        let next = SkillStore::from_json(&kept.last(), 100_000).unwrap();
        assert_eq!(next.changes("evening").len(), 2);
        assert_eq!(next.tidied().unwrap().summary, "Nothing needed tidying.");
        next.undo("evening").unwrap();
        assert!(next.load("evening").unwrap().body.contains("30%"));
    }

    #[test]
    fn the_set_outlasts_a_restart() {
        let kept = Arc::new(Kept::default());
        let store = store().with_sink(kept.clone());
        store
            .create(
                "evening",
                "Wind down",
                "0.1.0",
                "Dim the lights.",
                Provenance::AgentCreated,
            )
            .unwrap();
        store
            .create(
                "morning",
                "Wake up",
                "0.1.0",
                "Open the blinds.",
                Provenance::UserCreated,
            )
            .unwrap();
        store.set_pinned("morning", true).unwrap();
        store.bump_view("evening").unwrap();
        store.delete("evening", Some("morning")).unwrap();

        let next = SkillStore::from_json(&kept.last(), 100_000).unwrap();
        let morning = next.load("morning").unwrap();
        assert_eq!(morning.body, "Open the blinds.");
        assert!(morning.sidecar.pinned);
        assert!(matches!(
            next.load("evening").unwrap_err(),
            Error::Absorbed { .. }
        ));
    }

    #[test]
    fn a_refused_or_empty_change_is_not_handed_on() {
        let kept = Arc::new(Kept::default());
        let store = store().with_sink(kept.clone());
        let _ = store.create("Bad Name", "D", "0.1.0", "B", Provenance::UserCreated);
        let _ = store.patch("missing", "B");
        store.bump_view("missing").unwrap();
        store
            .create("s", "D", "0.1.0", "B", Provenance::AgentCreated)
            .unwrap();
        store.update_status("s", |_| None).unwrap();
        assert_eq!(kept.count(), 1);
    }

    #[test]
    fn a_kept_set_that_fails_the_checks_is_refused() {
        let document = serde_json::json!({ "skills": [{
            "name": "s", "description": "D", "version": "0.1.0",
            "body": "bad\u{200B}body",
            "sidecar": { "created_at": "2026-01-01T12:00:00Z", "provenance": "agent-created" },
        }]})
        .to_string();
        let err = SkillStore::from_json(&document, 100_000).err().unwrap();
        assert!(matches!(err, Error::ScanFailed { .. }), "{err:?}");
        assert!(SkillStore::from_json("not json", 100_000).is_err());
    }
}
