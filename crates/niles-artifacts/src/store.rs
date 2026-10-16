use crate::error::{Error, Result};
use crate::kinds::Preview;
use crate::model::{
    Artifact, Author, Changes, Filter, NewArtifact, NewRevision, PageChange, Revision,
    RevisionMeta, Status, StatusChange,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
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
    /// A new version with the content as it is now and `page` for its
    /// page. As it is when this is kept, not when the page was begun: a
    /// run Niles marked on a plan while its page was being written stays
    /// marked. An error for an artifact with nothing written yet, which
    /// has nothing to draw.
    async fn set_page(
        &self,
        person: &str,
        id: i64,
        page: PageChange,
        reason: String,
        author: Author,
    ) -> Result<Option<Artifact>>;
    /// The HTML of page `page` of an artifact of theirs, while a version
    /// they still have draws with it.
    async fn page(&self, person: &str, id: i64, page: i64) -> Result<Option<String>>;
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
/// person, with the page it had then: undoing a new page is restoring
/// the version before it. `None` when there is no such artifact or
/// version of theirs.
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
                page: old.page.map_or(PageChange::Remove, PageChange::Use),
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
    next_page: i64,
    /// Each page by its id, with the artifact it is of.
    pages: HashMap<i64, (i64, String)>,
}

impl Inner {
    fn find(&mut self, person: &str, id: i64) -> Option<&mut Artifact> {
        self.artifacts
            .iter_mut()
            .find(|a| a.id == id && a.person == person)
    }

    /// The page a new version of artifact `id` draws with, when the
    /// current one draws with `current`.
    fn page_for(
        &mut self,
        id: i64,
        current: Option<i64>,
        change: &PageChange,
    ) -> Result<Option<i64>> {
        match change {
            PageChange::Keep => Ok(current),
            PageChange::Remove => Ok(None),
            PageChange::New(html) => {
                self.next_page += 1;
                self.pages.insert(self.next_page, (id, html.clone()));
                Ok(Some(self.next_page))
            }
            PageChange::Use(page) => match self.pages.get(page) {
                Some((of, _)) if *of == id => Ok(Some(*page)),
                _ => Err(Error::Invalid {
                    kind: "page",
                    reason: format!("page {page} is not one this artifact keeps"),
                }),
            },
        }
    }

    /// A new version, numbered after the newest; the oldest past the
    /// limit goes, and with it any page no version draws with now.
    fn push_revision(
        &mut self,
        id: i64,
        content: Value,
        page: Option<i64>,
        reason: &str,
        author: Author,
    ) -> i32 {
        let kept = self.revisions.entry(id).or_default();
        let n = kept.last().map_or(1, |r| r.n + 1);
        kept.push(Revision {
            n,
            content,
            page,
            reason: reason.to_string(),
            author,
            at: Utc::now(),
        });
        let excess = kept.len().saturating_sub(KEPT_REVISIONS);
        kept.drain(..excess);
        let drawn: Vec<i64> = kept.iter().filter_map(|r| r.page).collect();
        self.pages
            .retain(|p, (of, _)| *of != id || drawn.contains(p));
        n
    }

    /// `content` kept as the new current version of artifact `id`.
    fn keep(
        &mut self,
        person: &str,
        id: i64,
        content: Value,
        change: &PageChange,
        reason: &str,
        author: Author,
    ) -> Result<Option<Artifact>> {
        let Some(current) = self.find(person, id).map(|a| a.page) else {
            return Ok(None);
        };
        let page = self.page_for(id, current, change)?;
        let preview =
            crate::kinds::preview(&self.find(person, id).expect("found above").kind, &content);
        let n = self.push_revision(id, content, page, reason, author);
        let artifact = self.find(person, id).expect("found above");
        artifact.current_revision = n;
        artifact.page = page;
        artifact.preview = preview;
        artifact.updated_at = Utc::now();
        Ok(Some(artifact.clone()))
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
            page: None,
        };
        let id = artifact.id;
        if let Some(summary) = new.first.as_ref().and_then(|f| f.summary.clone()) {
            artifact.summary = summary;
        }
        inner.artifacts.push(artifact.clone());
        let Some(first) = new.first else {
            return Ok(artifact);
        };
        match inner.keep(
            person,
            id,
            first.content,
            &first.page,
            &first.reason,
            first.author,
        ) {
            Ok(made) => Ok(made.expect("just made")),
            Err(e) => {
                inner.artifacts.retain(|a| a.id != id);
                Err(e)
            }
        }
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
        let Some(mut artifact) = inner.keep(
            person,
            id,
            revision.content,
            &revision.page,
            &revision.reason,
            revision.author,
        )?
        else {
            return Ok(None);
        };
        if let Some(summary) = revision.summary {
            artifact.summary = summary.clone();
            inner.find(person, id).expect("kept above").summary = summary;
        }
        Ok(Some(artifact))
    }

    async fn set_page(
        &self,
        person: &str,
        id: i64,
        page: PageChange,
        reason: String,
        author: Author,
    ) -> Result<Option<Artifact>> {
        let mut inner = self.lock();
        let Some(n) = inner.find(person, id).map(|a| a.current_revision) else {
            return Ok(None);
        };
        let content = inner
            .revisions
            .get(&id)
            .and_then(|r| r.iter().find(|r| r.n == n))
            .map(|r| r.content.clone())
            .ok_or_else(crate::page::nothing_to_draw)?;
        inner.keep(person, id, content, &page, &reason, author)
    }

    async fn page(&self, person: &str, id: i64, page: i64) -> Result<Option<String>> {
        let mut inner = self.lock();
        if inner.find(person, id).is_none() {
            return Ok(None);
        }
        Ok(inner
            .pages
            .get(&page)
            .filter(|(of, _)| *of == id)
            .map(|(_, html)| html.clone()))
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
            inner.pages.retain(|_, (of, _)| *of != id);
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
            page: PageChange::Keep,
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

    async fn with_page(store: &MemoryArtifacts, html: &str) -> Artifact {
        let a = store.create("person:mark", document("Plan")).await.unwrap();
        store
            .set_page(
                "person:mark",
                a.id,
                PageChange::New(html.into()),
                "A page for it".into(),
                Author::Claude,
            )
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn a_page_is_a_version_drawing_the_same_content() {
        let store = MemoryArtifacts::new();
        let a = with_page(&store, "<h1>Plan</h1>").await;
        assert_eq!(a.current_revision, 2);
        let page = a.page.expect("a page");
        let r = store
            .revision("person:mark", a.id, 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.content["markdown"], "# first");
        assert_eq!(r.page, Some(page));
        assert_eq!(r.reason, "A page for it");
        assert_eq!(
            store
                .page("person:mark", a.id, page)
                .await
                .unwrap()
                .as_deref(),
            Some("<h1>Plan</h1>")
        );
        let first = store
            .revision("person:mark", a.id, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.page, None, "the version before it had none");
    }

    #[tokio::test]
    async fn changing_the_data_keeps_the_page_without_copying_it() {
        let store = MemoryArtifacts::new();
        let a = with_page(&store, "<h1>Plan</h1>").await;
        for i in 0..5 {
            let kept = store
                .revise(
                    "person:mark",
                    a.id,
                    version(&format!("v{i}"), Author::Niles),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(kept.page, a.page);
        }
        assert_eq!(
            store.lock().pages.len(),
            1,
            "one page, however many versions"
        );
        let now = store.get("person:mark", a.id).await.unwrap().unwrap();
        assert_eq!(now.page, a.page);
        assert_eq!(now.preview.lines[0].text, "v4");
    }

    #[tokio::test]
    async fn a_page_is_set_over_the_content_as_it_is_when_kept() {
        let store = MemoryArtifacts::new();
        let a = store.create("person:mark", document("Plan")).await.unwrap();
        // A run marked while the page was being written.
        store
            .revise("person:mark", a.id, version("marked", Author::Niles))
            .await
            .unwrap();
        let paged = store
            .set_page(
                "person:mark",
                a.id,
                PageChange::New("<p>x</p>".into()),
                "Page".into(),
                Author::Claude,
            )
            .await
            .unwrap()
            .unwrap();
        let r = store
            .revision("person:mark", a.id, paged.current_revision)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.content["markdown"], "marked");
    }

    #[tokio::test]
    async fn removing_the_page_and_restoring_brings_it_back() {
        let store = MemoryArtifacts::new();
        let a = with_page(&store, "<h1>Plan</h1>").await;
        let gone = store
            .set_page(
                "person:mark",
                a.id,
                PageChange::Remove,
                "Took the page away".into(),
                Author::Person,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(gone.page, None);
        let back = restore(&store, "person:mark", a.id, a.current_revision)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(back.page, a.page);
        let before = restore(&store, "person:mark", a.id, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.page, None, "the first version had none");
    }

    #[tokio::test]
    async fn a_page_goes_once_no_kept_version_draws_with_it() {
        let store = MemoryArtifacts::new();
        let a = with_page(&store, "<h1>old</h1>").await;
        let old = a.page.unwrap();
        let new = store
            .set_page(
                "person:mark",
                a.id,
                PageChange::New("<h1>new</h1>".into()),
                "A new page".into(),
                Author::Claude,
            )
            .await
            .unwrap()
            .unwrap();
        for i in 0..KEPT_REVISIONS {
            store
                .revise(
                    "person:mark",
                    a.id,
                    version(&format!("v{i}"), Author::Niles),
                )
                .await
                .unwrap();
        }
        assert!(
            store
                .page("person:mark", a.id, old)
                .await
                .unwrap()
                .is_none()
        );
        let kept = new.page.unwrap();
        assert!(
            store
                .page("person:mark", a.id, kept)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(store.lock().pages.len(), 1);
    }

    #[tokio::test]
    async fn a_page_needs_something_written_and_is_never_another_persons() {
        let store = MemoryArtifacts::new();
        let mut job = document("Being written");
        job.first = None;
        let empty = store.create("person:mark", job).await.unwrap();
        let e = store
            .set_page(
                "person:mark",
                empty.id,
                PageChange::New("<p>x</p>".into()),
                "Page".into(),
                Author::Claude,
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("nothing is written"), "{e}");

        let a = with_page(&store, "<h1>his</h1>").await;
        let page = a.page.unwrap();
        let her = "person:majse";
        assert!(store.page(her, a.id, page).await.unwrap().is_none());
        assert!(
            store
                .set_page(her, a.id, PageChange::Remove, "x".into(), Author::Person)
                .await
                .unwrap()
                .is_none()
        );
        // A page id of another artifact is not this one's to use.
        let other = store
            .create("person:mark", document("Other"))
            .await
            .unwrap();
        let e = store
            .revise(
                "person:mark",
                other.id,
                NewRevision {
                    page: PageChange::Use(page),
                    ..version("x", Author::Person)
                },
            )
            .await;
        assert!(e.is_err());
        assert!(store.delete("person:mark", a.id).await.unwrap());
        assert!(store.lock().pages.is_empty(), "deleting takes its pages");
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
