use crate::error::Result;
use crate::kinds::Preview;
use crate::model::{
    Artifact, Author, Changes, Filter, NewArtifact, NewRevision, Revision, RevisionMeta, Status,
    StatusChange,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Mutex;

/// How many versions an artifact keeps. The oldest goes as a new one is
/// saved.
pub const KEPT_REVISIONS: usize = 50;

/// Every person's artifacts. Each call names the person, and finds only
/// theirs: another person's id is the same as no id at all, so a bug
/// upstream that passes the wrong id reads nothing.
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn create(&self, person: &str, new: NewArtifact) -> Result<Artifact>;
    /// Pinned first, then the most recently changed.
    async fn list(&self, person: &str, filter: Filter) -> Result<Vec<Artifact>>;
    async fn get(&self, person: &str, id: i64) -> Result<Option<Artifact>>;
    async fn revision(&self, person: &str, id: i64, n: i32) -> Result<Option<Revision>>;
    /// Newest first.
    async fn revisions(&self, person: &str, id: i64) -> Result<Vec<RevisionMeta>>;
    /// A new version, which becomes the current one.
    async fn revise(
        &self,
        person: &str,
        id: i64,
        revision: NewRevision,
    ) -> Result<Option<Artifact>>;
    async fn change(&self, person: &str, id: i64, changes: Changes) -> Result<Option<Artifact>>;
    async fn set_status(
        &self,
        person: &str,
        id: i64,
        status: StatusChange,
    ) -> Result<Option<Artifact>>;
    /// The chat a job's card goes into, once the turn that started it is
    /// kept and has one.
    async fn set_conversation(&self, person: &str, id: i64, conversation: i64) -> Result<bool>;
    /// The artifact and every version of it.
    async fn delete(&self, person: &str, id: i64) -> Result<bool>;
    /// Every job still `working` that last moved before `before`,
    /// whoever's, marked failed with `reason`: after a restart, nothing
    /// is writing them any more. One started since is this run's own.
    async fn fail_abandoned(&self, reason: &str, before: DateTime<Utc>) -> Result<u64>;
}

/// Version `n` of an artifact saved again as a new version, by the
/// person. `None` when there is no such artifact or version of theirs.
pub async fn restore(
    store: &dyn ArtifactStore,
    person: &str,
    id: i64,
    n: i32,
) -> Result<Option<Artifact>> {
    let Some(old) = store.revision(person, id, n).await? else {
        return Ok(None);
    };
    store
        .revise(
            person,
            id,
            NewRevision {
                content: old.content,
                reason: format!("Restored version {n}"),
                author: Author::Person,
                summary: None,
            },
        )
        .await
}

/// Artifacts in memory, for tests.
#[derive(Default)]
pub struct MemoryArtifacts {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    next: i64,
    artifacts: Vec<Artifact>,
    revisions: HashMap<i64, Vec<Revision>>,
}

impl Inner {
    fn find(&mut self, person: &str, id: i64) -> Option<&mut Artifact> {
        self.artifacts
            .iter_mut()
            .find(|a| a.id == id && a.person == person)
    }

    fn push_revision(&mut self, id: i64, revision: &NewRevision) -> i32 {
        let kept = self.revisions.entry(id).or_default();
        let n = kept.last().map_or(1, |r| r.n + 1);
        kept.push(Revision {
            n,
            content: revision.content.clone(),
            reason: revision.reason.clone(),
            author: revision.author,
            at: Utc::now(),
        });
        let excess = kept.len().saturating_sub(KEPT_REVISIONS);
        kept.drain(..excess);
        n
    }
}

impl MemoryArtifacts {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl ArtifactStore for MemoryArtifacts {
    async fn create(&self, person: &str, new: NewArtifact) -> Result<Artifact> {
        let mut inner = self.lock();
        inner.next += 1;
        let now = Utc::now();
        let mut artifact = Artifact {
            id: inner.next,
            person: person.to_string(),
            kind: new.kind,
            title: new.title,
            summary: new.summary,
            pinned: false,
            archived: false,
            notify: new.notify,
            status: new.status,
            status_text: new.status_text,
            error: None,
            job_id: new.job_id,
            brief: new.brief,
            conversation_id: new.conversation_id,
            created_at: now,
            updated_at: now,
            current_revision: 0,
            preview: Preview::default(),
        };
        if let Some(first) = &new.first {
            artifact.current_revision = inner.push_revision(artifact.id, first);
            artifact.preview = crate::kinds::preview(&artifact.kind, &first.content);
            if let Some(summary) = &first.summary {
                artifact.summary = summary.clone();
            }
        }
        inner.artifacts.push(artifact.clone());
        Ok(artifact)
    }

    async fn list(&self, person: &str, filter: Filter) -> Result<Vec<Artifact>> {
        let mut out: Vec<Artifact> = self
            .lock()
            .artifacts
            .iter()
            .filter(|a| a.person == person && filter.admits(a))
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated_at.cmp(&a.updated_at))
                .then(b.id.cmp(&a.id))
        });
        Ok(out)
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<Artifact>> {
        Ok(self.lock().find(person, id).cloned())
    }

    async fn revision(&self, person: &str, id: i64, n: i32) -> Result<Option<Revision>> {
        let mut inner = self.lock();
        if inner.find(person, id).is_none() {
            return Ok(None);
        }
        Ok(inner
            .revisions
            .get(&id)
            .and_then(|r| r.iter().find(|r| r.n == n))
            .cloned())
    }

    async fn revisions(&self, person: &str, id: i64) -> Result<Vec<RevisionMeta>> {
        let mut inner = self.lock();
        if inner.find(person, id).is_none() {
            return Ok(Vec::new());
        }
        Ok(inner
            .revisions
            .get(&id)
            .map(|r| r.iter().rev().map(RevisionMeta::from).collect())
            .unwrap_or_default())
    }

    async fn revise(
        &self,
        person: &str,
        id: i64,
        revision: NewRevision,
    ) -> Result<Option<Artifact>> {
        let mut inner = self.lock();
        if inner.find(person, id).is_none() {
            return Ok(None);
        }
        let n = inner.push_revision(id, &revision);
        let artifact = inner.find(person, id).expect("found above");
        artifact.current_revision = n;
        artifact.preview = crate::kinds::preview(&artifact.kind, &revision.content);
        artifact.updated_at = Utc::now();
        if let Some(summary) = revision.summary {
            artifact.summary = summary;
        }
        Ok(Some(artifact.clone()))
    }

    async fn change(&self, person: &str, id: i64, changes: Changes) -> Result<Option<Artifact>> {
        let mut inner = self.lock();
        let Some(a) = inner.find(person, id) else {
            return Ok(None);
        };
        if let Some(title) = changes.title {
            a.title = title;
            a.updated_at = Utc::now();
        }
        if let Some(pinned) = changes.pinned {
            a.pinned = pinned;
        }
        if let Some(archived) = changes.archived {
            a.archived = archived;
        }
        if let Some(notify) = changes.notify {
            a.notify = notify;
        }
        Ok(Some(a.clone()))
    }

    async fn set_status(
        &self,
        person: &str,
        id: i64,
        status: StatusChange,
    ) -> Result<Option<Artifact>> {
        let mut inner = self.lock();
        let Some(a) = inner.find(person, id) else {
            return Ok(None);
        };
        a.status = status.status;
        a.status_text = status.status_text;
        a.error = status.error;
        a.job_id = status.job_id;
        a.updated_at = Utc::now();
        Ok(Some(a.clone()))
    }

    async fn set_conversation(&self, person: &str, id: i64, conversation: i64) -> Result<bool> {
        let mut inner = self.lock();
        let Some(a) = inner.find(person, id) else {
            return Ok(false);
        };
        a.conversation_id = Some(conversation);
        Ok(true)
    }

    async fn delete(&self, person: &str, id: i64) -> Result<bool> {
        let mut inner = self.lock();
        let before = inner.artifacts.len();
        inner
            .artifacts
            .retain(|a| !(a.id == id && a.person == person));
        let gone = inner.artifacts.len() != before;
        if gone {
            inner.revisions.remove(&id);
        }
        Ok(gone)
    }

    async fn fail_abandoned(&self, reason: &str, before: DateTime<Utc>) -> Result<u64> {
        let mut inner = self.lock();
        let mut n = 0;
        for a in inner
            .artifacts
            .iter_mut()
            .filter(|a| a.status == Status::Working && a.updated_at < before)
        {
            a.status = Status::Failed;
            a.status_text = None;
            a.error = Some(reason.to_string());
            a.job_id = None;
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document(title: &str) -> NewArtifact {
        NewArtifact {
            kind: "document".into(),
            title: title.into(),
            summary: String::new(),
            notify: true,
            status: Status::Ready,
            status_text: None,
            job_id: None,
            brief: None,
            conversation_id: None,
            first: Some(version("# first", Author::Claude)),
        }
    }

    fn version(markdown: &str, author: Author) -> NewRevision {
        NewRevision {
            content: json!({ "markdown": markdown }),
            reason: "because".into(),
            author,
            summary: None,
        }
    }

    #[tokio::test]
    async fn a_new_artifact_with_content_starts_at_version_one() {
        let store = MemoryArtifacts::new();
        let a = store
            .create("person:mark", document("Packing list"))
            .await
            .unwrap();
        assert_eq!(a.current_revision, 1);
        let r = store
            .revision("person:mark", a.id, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.content["markdown"], "# first");
        assert_eq!(r.author, Author::Claude);
    }

    #[tokio::test]
    async fn only_the_last_fifty_versions_are_kept() {
        let store = MemoryArtifacts::new();
        let a = store.create("person:mark", document("Plan")).await.unwrap();
        for i in 0..60 {
            store
                .revise(
                    "person:mark",
                    a.id,
                    version(&format!("v{i}"), Author::Claude),
                )
                .await
                .unwrap();
        }
        let kept = store.revisions("person:mark", a.id).await.unwrap();
        assert_eq!(kept.len(), KEPT_REVISIONS);
        assert_eq!(kept[0].n, 61, "newest first");
        assert_eq!(kept.last().unwrap().n, 12);
        assert!(
            store
                .revision("person:mark", a.id, 1)
                .await
                .unwrap()
                .is_none()
        );
        let current = store.get("person:mark", a.id).await.unwrap().unwrap();
        assert_eq!(current.current_revision, 61);
    }

    #[tokio::test]
    async fn restoring_a_version_saves_it_as_a_new_one() {
        let store = MemoryArtifacts::new();
        let a = store.create("person:mark", document("Plan")).await.unwrap();
        store
            .revise("person:mark", a.id, version("second", Author::Claude))
            .await
            .unwrap();
        let restored = restore(&store, "person:mark", a.id, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.current_revision, 3);
        let r = store
            .revision("person:mark", a.id, 3)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.content["markdown"], "# first");
        assert_eq!(r.reason, "Restored version 1");
        assert_eq!(r.author, Author::Person);
        assert!(
            restore(&store, "person:mark", a.id, 9)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn another_persons_artifact_is_not_there() {
        let store = MemoryArtifacts::new();
        let a = store.create("person:mark", document("His")).await.unwrap();
        let her = "person:majse";
        assert!(store.get(her, a.id).await.unwrap().is_none());
        assert!(store.list(her, Filter::default()).await.unwrap().is_empty());
        assert!(store.revision(her, a.id, 1).await.unwrap().is_none());
        assert!(store.revisions(her, a.id).await.unwrap().is_empty());
        assert!(
            store
                .revise(her, a.id, version("mine now", Author::Person))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .change(
                    her,
                    a.id,
                    Changes {
                        title: Some("Hers".into()),
                        ..Changes::default()
                    }
                )
                .await
                .unwrap()
                .is_none()
        );
        assert!(!store.delete(her, a.id).await.unwrap());
        assert!(restore(&store, her, a.id, 1).await.unwrap().is_none());
        let still = store.get("person:mark", a.id).await.unwrap().unwrap();
        assert_eq!(still.title, "His");
        assert_eq!(still.current_revision, 1);
    }

    #[tokio::test]
    async fn the_list_is_pinned_first_then_newest() {
        let store = MemoryArtifacts::new();
        let p = "person:mark";
        let old = store.create(p, document("Old")).await.unwrap();
        let new = store.create(p, document("New")).await.unwrap();
        let archived = store.create(p, document("Gone")).await.unwrap();
        store
            .change(
                p,
                archived.id,
                Changes {
                    archived: Some(true),
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        store
            .change(
                p,
                old.id,
                Changes {
                    pinned: Some(true),
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        let shown = store
            .list(
                p,
                Filter {
                    archived: Some(false),
                    ..Filter::default()
                },
            )
            .await
            .unwrap();
        let ids: Vec<i64> = shown.iter().map(|a| a.id).collect();
        assert_eq!(ids, vec![old.id, new.id]);
        let all = store.list(p, Filter::default()).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn a_restart_fails_every_job_that_was_running() {
        let store = MemoryArtifacts::new();
        let mut job = document("Plan");
        job.first = None;
        job.status = Status::Working;
        job.status_text = Some("Writing…".into());
        let a = store.create("person:mark", job.clone()).await.unwrap();
        let restarted = Utc::now() + chrono::Duration::milliseconds(1);
        // Started by this run, after it came up: still its own.
        let mut later = store.create("person:mark", job).await.unwrap();
        later.updated_at = restarted + chrono::Duration::seconds(1);
        store
            .lock()
            .find("person:mark", later.id)
            .unwrap()
            .updated_at = later.updated_at;

        assert_eq!(
            store
                .fail_abandoned("Niles restarted", restarted)
                .await
                .unwrap(),
            1
        );
        let a = store.get("person:mark", a.id).await.unwrap().unwrap();
        assert_eq!(a.status, Status::Failed);
        assert_eq!(a.status_text, None);
        assert_eq!(a.error.as_deref(), Some("Niles restarted"));
        let later = store.get("person:mark", later.id).await.unwrap().unwrap();
        assert_eq!(later.status, Status::Working);
    }

    #[tokio::test]
    async fn deleting_takes_every_version_with_it() {
        let store = MemoryArtifacts::new();
        let a = store.create("person:mark", document("Plan")).await.unwrap();
        assert!(store.delete("person:mark", a.id).await.unwrap());
        assert!(store.get("person:mark", a.id).await.unwrap().is_none());
        assert!(
            store
                .revisions("person:mark", a.id)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
