use crate::error::{Error, Result};
use crate::images::{Image, ImageData, NewImage};
use crate::kinds::Preview;
use crate::model::{
    Artifact, Author, Changes, Filter, NewArtifact, NewRevision, PageChange, Revision,
    RevisionMeta, Status, StatusChange,
};
use crate::share::{Level, Share};
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
/// upstream that passes the wrong id reads nothing. What is shared with
/// a person is found only by the calls that say so ([`Self::shared`],
/// [`Self::shared_with`]), and changed under its owner's key once
/// [`crate::share::open`] has said they may.
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn create(&self, person: &str, new: NewArtifact) -> Result<Artifact>;
    /// Pinned first, then the most recently changed.
    async fn list(&self, person: &str, filter: Filter) -> Result<Vec<Artifact>>;
    async fn get(&self, person: &str, id: i64) -> Result<Option<Artifact>>;
    async fn revision(&self, person: &str, id: i64, n: i32) -> Result<Option<Revision>>;
    /// Newest first.
    async fn revisions(&self, person: &str, id: i64) -> Result<Vec<RevisionMeta>>;
    /// A new version, which becomes the current one. When `expected` is
    /// given, refuse unless it is still current, checked under the write
    /// lock. Partial edits use the version they read; whole saves use None.
    async fn revise(
        &self,
        person: &str,
        id: i64,
        revision: NewRevision,
        expected: Option<i32>,
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
        by: Option<String>,
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
    /// A picture kept for artifact `id` of theirs, if it fits beside the
    /// ones it keeps ([`crate::images::check_room`]); `None` when there is
    /// no such artifact of theirs.
    async fn add_image(&self, person: &str, id: i64, image: NewImage) -> Result<Option<Image>>;
    /// The pictures artifact `id` of theirs keeps, oldest first.
    async fn images(&self, person: &str, id: i64) -> Result<Vec<Image>>;
    /// Picture `image` of artifact `id` of theirs, with its bytes.
    async fn image(&self, person: &str, id: i64, image: i64) -> Result<Option<ImageData>>;
    /// Whether there was such a picture of theirs to take away. Content
    /// that names it is left as it is, and shows nothing there.
    async fn remove_image(&self, person: &str, id: i64, image: i64) -> Result<bool>;
    /// The artifact, every version of it, its pages and pictures, and
    /// whoever it was shared with.
    async fn delete(&self, person: &str, id: i64) -> Result<bool>;
    /// Artifact `id` of someone else's that is shared with `person`,
    /// with `person`'s own pin, and how it is shared.
    async fn shared(&self, person: &str, id: i64) -> Result<Option<(Artifact, Level)>>;
    /// Everything of others' shared with `person`, each with their own
    /// pin, ordered as [`Self::list`] orders.
    async fn shared_with(&self, person: &str, filter: Filter) -> Result<Vec<(Artifact, Level)>>;
    /// Who artifact `id` of `owner`'s is shared with, in the order they
    /// were shared with; nobody for an artifact that is not theirs.
    async fn shares(&self, owner: &str, id: i64) -> Result<Vec<Share>>;
    /// Share artifact `id` of `owner`'s with `person` at `level`, or a
    /// new level; with `None`, stop. False when there is no such
    /// artifact of `owner`'s. Someone shared with keeps when they were
    /// first shared with, and their pin, through a change of level. It
    /// is refused for the owner themselves.
    async fn set_share(
        &self,
        owner: &str,
        id: i64,
        person: &str,
        level: Option<Level>,
    ) -> Result<bool>;
    /// `person`'s own pin on an artifact shared with them. False when it
    /// is not.
    async fn pin_shared(&self, person: &str, id: i64, pinned: bool) -> Result<bool>;
    /// Every job still `working` that last moved before `before`,
    /// whoever's, marked failed with `reason`: after a restart, nothing
    /// is writing them any more. One started since is this run's own.
    async fn fail_abandoned(&self, reason: &str, before: DateTime<Utc>) -> Result<u64>;
    /// Every artifact of `kind` made before `before`, whoever's, oldest
    /// first: what a kind kept only for a while has outlived.
    async fn made_before(&self, kind: &str, before: DateTime<Utc>) -> Result<Vec<Artifact>>;
}

/// Version `n` of an artifact of `owner`'s saved again as a new version,
/// by person `by` (the owner, or someone it is shared with to edit),
/// with the page it had then: undoing a new page is restoring the
/// version before it. `None` when there is no such artifact or version
/// of `owner`'s.
pub async fn restore(
    store: &dyn ArtifactStore,
    owner: &str,
    id: i64,
    n: i32,
    by: &str,
) -> Result<Option<Artifact>> {
    let Some(old) = store.revision(owner, id, n).await? else {
        return Ok(None);
    };
    store
        .revise(
            owner,
            id,
            NewRevision {
                content: old.content,
                reason: format!("Restored version {n}"),
                author: Author::Person,
                by: Some(by.to_string()),
                summary: None,
                page: old.page.map_or(PageChange::Remove, PageChange::Use),
            },
            None,
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
    next_image: i64,
    /// Each picture with the artifact it is of, oldest first.
    images: Vec<(i64, ImageData)>,
    /// Who each artifact is shared with, oldest share first.
    shares: Vec<Shared>,
}

struct Shared {
    artifact: i64,
    share: Share,
    pinned: bool,
}

impl Inner {
    fn find(&mut self, person: &str, id: i64) -> Option<&mut Artifact> {
        self.artifacts
            .iter_mut()
            .find(|a| a.id == id && a.person == person)
    }

    /// Artifact `id` as `person`, who it is shared with, has it.
    fn shared(&self, person: &str, id: i64) -> Option<(Artifact, Level)> {
        let row = self
            .shares
            .iter()
            .find(|s| s.artifact == id && s.share.person == person)?;
        let mut artifact = self.artifacts.iter().find(|a| a.id == id)?.clone();
        artifact.pinned = row.pinned;
        Some((artifact, row.share.level))
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
            PageChange::Edit { from, .. } if current != Some(*from) => Err(Error::Invalid {
                kind: "edit",
                reason:
                    "the page changed while these edits were being made; read it again and retry"
                        .into(),
            }),
            PageChange::New(html) | PageChange::Edit { html, .. } => {
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
        by: Option<String>,
    ) -> i32 {
        let kept = self.revisions.entry(id).or_default();
        let n = kept.last().map_or(1, |r| r.n + 1);
        kept.push(Revision {
            n,
            content,
            page,
            reason: reason.to_string(),
            author,
            by,
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
    #[allow(clippy::too_many_arguments)]
    fn keep(
        &mut self,
        person: &str,
        id: i64,
        content: Value,
        change: &PageChange,
        reason: &str,
        author: Author,
        by: Option<String>,
    ) -> Result<Option<Artifact>> {
        let Some(current) = self.find(person, id).map(|a| a.page) else {
            return Ok(None);
        };
        let page = self.page_for(id, current, change)?;
        let preview =
            crate::kinds::preview(&self.find(person, id).expect("found above").kind, &content);
        let n = self.push_revision(id, content, page, reason, author, by.clone());
        let artifact = self.find(person, id).expect("found above");
        artifact.current_revision = n;
        artifact.page = page;
        artifact.preview = preview;
        artifact.changed_by = by;
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
            changed_by: None,
            shared_with: Vec::new(),
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
            first.by,
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
        expected: Option<i32>,
    ) -> Result<Option<Artifact>> {
        let mut inner = self.lock();
        let Some(current) = inner.find(person, id).map(|a| a.current_revision) else {
            return Ok(None);
        };
        if expected.is_some_and(|n| n != current) {
            return Err(Error::Invalid {
                kind: "edit",
                reason: "the artifact changed while these edits were being made; read it again and retry"
                    .into(),
            });
        }
        let Some(mut artifact) = inner.keep(
            person,
            id,
            revision.content,
            &revision.page,
            &revision.reason,
            revision.author,
            revision.by,
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
        by: Option<String>,
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
        inner.keep(person, id, content, &page, &reason, author, by)
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

    async fn add_image(&self, person: &str, id: i64, image: NewImage) -> Result<Option<Image>> {
        let mut inner = self.lock();
        if inner.find(person, id).is_none() {
            return Ok(None);
        }
        let kept: Vec<usize> = inner
            .images
            .iter()
            .filter(|(of, _)| *of == id)
            .map(|(_, i)| i.bytes.len())
            .collect();
        crate::images::check_room(kept.len(), kept.iter().sum(), image.picture.bytes.len())?;
        inner.next_image += 1;
        let made = Image {
            id: inner.next_image,
            mime: image.picture.mime.to_string(),
            width: image.picture.width,
            height: image.picture.height,
            size: image.picture.bytes.len(),
            alt: image.alt,
            credit: image.credit,
            source: image.source,
            created_at: Utc::now(),
        };
        inner.images.push((
            id,
            ImageData {
                image: made.clone(),
                bytes: image.picture.bytes,
            },
        ));
        Ok(Some(made))
    }

    async fn images(&self, person: &str, id: i64) -> Result<Vec<Image>> {
        let mut inner = self.lock();
        if inner.find(person, id).is_none() {
            return Ok(Vec::new());
        }
        Ok(inner
            .images
            .iter()
            .filter(|(of, _)| *of == id)
            .map(|(_, i)| i.image.clone())
            .collect())
    }

    async fn image(&self, person: &str, id: i64, image: i64) -> Result<Option<ImageData>> {
        let mut inner = self.lock();
        if inner.find(person, id).is_none() {
            return Ok(None);
        }
        Ok(inner
            .images
            .iter()
            .find(|(of, i)| *of == id && i.image.id == image)
            .map(|(_, i)| i.clone()))
    }

    async fn remove_image(&self, person: &str, id: i64, image: i64) -> Result<bool> {
        let mut inner = self.lock();
        if inner.find(person, id).is_none() {
            return Ok(false);
        }
        let before = inner.images.len();
        inner
            .images
            .retain(|(of, i)| !(*of == id && i.image.id == image));
        Ok(inner.images.len() != before)
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
            inner.images.retain(|(of, _)| *of != id);
            inner.shares.retain(|s| s.artifact != id);
        }
        Ok(gone)
    }

    async fn shared(&self, person: &str, id: i64) -> Result<Option<(Artifact, Level)>> {
        Ok(self.lock().shared(person, id))
    }

    async fn shared_with(&self, person: &str, filter: Filter) -> Result<Vec<(Artifact, Level)>> {
        let inner = self.lock();
        let mut out: Vec<(Artifact, Level)> = inner
            .shares
            .iter()
            .filter(|s| s.share.person == person)
            .filter_map(|s| inner.shared(person, s.artifact))
            .filter(|(a, _)| filter.admits(a))
            .collect();
        out.sort_by(|(a, _), (b, _)| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated_at.cmp(&a.updated_at))
                .then(b.id.cmp(&a.id))
        });
        Ok(out)
    }

    async fn shares(&self, owner: &str, id: i64) -> Result<Vec<Share>> {
        let mut inner = self.lock();
        if inner.find(owner, id).is_none() {
            return Ok(Vec::new());
        }
        Ok(inner
            .shares
            .iter()
            .filter(|s| s.artifact == id)
            .map(|s| s.share.clone())
            .collect())
    }

    async fn set_share(
        &self,
        owner: &str,
        id: i64,
        person: &str,
        level: Option<Level>,
    ) -> Result<bool> {
        let mut inner = self.lock();
        if inner.find(owner, id).is_none() {
            return Ok(false);
        }
        if person == owner {
            return Err(crate::share::with_the_owner());
        }
        let at = inner
            .shares
            .iter()
            .position(|s| s.artifact == id && s.share.person == person);
        match (level, at) {
            (Some(level), Some(at)) => inner.shares[at].share.level = level,
            (Some(level), None) => inner.shares.push(Shared {
                artifact: id,
                share: Share {
                    person: person.to_string(),
                    level,
                    shared_at: Utc::now(),
                },
                pinned: false,
            }),
            (None, Some(at)) => {
                inner.shares.remove(at);
            }
            (None, None) => {}
        }
        let with: Vec<String> = inner
            .shares
            .iter()
            .filter(|s| s.artifact == id)
            .map(|s| s.share.person.clone())
            .collect();
        inner.find(owner, id).expect("found above").shared_with = with;
        Ok(true)
    }

    async fn pin_shared(&self, person: &str, id: i64, pinned: bool) -> Result<bool> {
        let mut inner = self.lock();
        let Some(row) = inner
            .shares
            .iter_mut()
            .find(|s| s.artifact == id && s.share.person == person)
        else {
            return Ok(false);
        };
        row.pinned = pinned;
        Ok(true)
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

    async fn made_before(&self, kind: &str, before: DateTime<Utc>) -> Result<Vec<Artifact>> {
        let mut out: Vec<Artifact> = self
            .lock()
            .artifacts
            .iter()
            .filter(|a| a.kind == kind && a.created_at < before)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(out)
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
            by: None,
            summary: None,
            page: PageChange::Keep,
        }
    }

    #[tokio::test]
    async fn stale_content_edits_cannot_discard_another_edit() {
        let store = MemoryArtifacts::new();
        let mut new = document("Plan");
        new.first = Some(version("alpha beta", Author::Claude));
        let a = store.create("person:mark", new).await.unwrap();
        let base = store
            .revision("person:mark", a.id, a.current_revision)
            .await
            .unwrap()
            .unwrap();
        let edit = |old, new| {
            let mut revision = version("", Author::Claude);
            revision.content = crate::edit::content(
                "document",
                &base.content,
                &json!([{ "old": old, "new": new }]),
            )
            .unwrap();
            revision
        };
        let first = edit("alpha", "ALPHA");
        let second = edit("beta", "BETA");
        let kept = store
            .revise("person:mark", a.id, first, Some(base.n))
            .await
            .unwrap()
            .unwrap();
        assert!(
            store
                .revise("person:mark", a.id, second.clone(), Some(base.n))
                .await
                .is_err()
        );
        let now = store.get("person:mark", a.id).await.unwrap().unwrap();
        assert_eq!(now.current_revision, kept.current_revision);
        let current = store
            .revision("person:mark", a.id, now.current_revision)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.content["markdown"], "ALPHA beta");
        let replaced = store
            .revise("person:mark", a.id, second, None)
            .await
            .unwrap()
            .unwrap();
        let current = store
            .revision("person:mark", a.id, replaced.current_revision)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.content["markdown"], "alpha BETA");
    }

    #[tokio::test]
    async fn stale_page_edits_cannot_discard_another_edit() {
        let store = MemoryArtifacts::new();
        let a = with_page(&store, "<p>alpha beta</p>").await;
        let from = a.page.unwrap();
        let base = store
            .page("person:mark", a.id, from)
            .await
            .unwrap()
            .unwrap();
        let edit = |old, new| PageChange::Edit {
            from,
            html: crate::edit::text(&base, &json!([{ "old": old, "new": new }])).unwrap(),
        };
        let first = edit("alpha", "ALPHA");
        let second = edit("beta", "BETA");
        // A content change does not stale a page edit, and must survive it.
        store
            .revise("person:mark", a.id, version("marked", Author::Niles), None)
            .await
            .unwrap();
        let kept = store
            .set_page(
                "person:mark",
                a.id,
                first,
                "Edit".into(),
                Author::Claude,
                None,
            )
            .await
            .unwrap()
            .unwrap();
        assert!(
            store
                .set_page(
                    "person:mark",
                    a.id,
                    second,
                    "Edit".into(),
                    Author::Claude,
                    None
                )
                .await
                .is_err()
        );
        let now = store.get("person:mark", a.id).await.unwrap().unwrap();
        assert_eq!(now.current_revision, kept.current_revision);
        assert_eq!(
            store
                .page("person:mark", a.id, now.page.unwrap())
                .await
                .unwrap()
                .unwrap(),
            "<p>ALPHA beta</p>"
        );
        let current = store
            .revision("person:mark", a.id, now.current_revision)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.content["markdown"], "marked");
        store
            .set_page(
                "person:mark",
                a.id,
                PageChange::Remove,
                "Remove".into(),
                Author::Person,
                None,
            )
            .await
            .unwrap();
        assert!(
            store
                .set_page(
                    "person:mark",
                    a.id,
                    PageChange::Edit {
                        from: kept.page.unwrap(),
                        html: "<p>new</p>".into()
                    },
                    "Edit".into(),
                    Author::Claude,
                    None,
                )
                .await
                .is_err()
        );
        let now = store.get("person:mark", a.id).await.unwrap().unwrap();
        assert_eq!(now.page, None);
        assert_eq!(now.current_revision, kept.current_revision + 1);
    }

    #[tokio::test]
    async fn made_before_finds_only_that_kind_made_before_then_whoever_s() {
        let store = MemoryArtifacts::new();
        let mark = store
            .create("person:mark", document("Old document"))
            .await
            .unwrap();
        let mut hers = document("Her document");
        hers.kind = "document".into();
        let majse = store.create("person:majse", hers).await.unwrap();
        let mut pod = document("Episode one");
        pod.kind = "podcast".into();
        let episode = store.create("person:mark", pod).await.unwrap();
        let cutoff = Utc::now() + chrono::Duration::days(1);
        let found = store.made_before("podcast", cutoff).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, episode.id);
        assert_eq!(found[0].person, "person:mark");
        // Too recent to be outlived, and other kinds never are.
        assert!(
            store
                .made_before("podcast", Utc::now() - chrono::Duration::seconds(1))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.made_before("document", cutoff).await.unwrap().len(),
            2
        );
        // Sorting is oldest first across owners.
        let docs = store.made_before("document", cutoff).await.unwrap();
        assert_eq!(docs[0].id, mark.id.min(majse.id));
        let _ = (mark, majse);
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
                    None,
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
            .revise("person:mark", a.id, version("second", Author::Claude), None)
            .await
            .unwrap();
        let restored = restore(&store, "person:mark", a.id, 1, "person:mark")
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
            restore(&store, "person:mark", a.id, 9, "person:mark")
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
                .revise(her, a.id, version("mine now", Author::Person), None)
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
        assert!(restore(&store, her, a.id, 1, her).await.unwrap().is_none());
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
                None,
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
                    None,
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
            .revise("person:mark", a.id, version("marked", Author::Niles), None)
            .await
            .unwrap();
        let paged = store
            .set_page(
                "person:mark",
                a.id,
                PageChange::New("<p>x</p>".into()),
                "Page".into(),
                Author::Claude,
                None,
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
                None,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(gone.page, None);
        let back = restore(
            &store,
            "person:mark",
            a.id,
            a.current_revision,
            "person:mark",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(back.page, a.page);
        let before = restore(&store, "person:mark", a.id, 1, "person:mark")
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
                None,
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
                    None,
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
                None,
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
                .set_page(
                    her,
                    a.id,
                    PageChange::Remove,
                    "x".into(),
                    Author::Person,
                    None
                )
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
                None,
            )
            .await;
        assert!(e.is_err());
        assert!(store.delete("person:mark", a.id).await.unwrap());
        assert!(store.lock().pages.is_empty(), "deleting takes its pages");
    }

    fn picture(alt: &str) -> NewImage {
        let picture = crate::images::prepare(&crate::images::tests::jpeg(8, 6)).unwrap();
        NewImage::new(picture, alt, Some("Photo: J. Doe"), None).unwrap()
    }

    #[tokio::test]
    async fn a_picture_is_kept_listed_served_and_taken_away() {
        let store = MemoryArtifacts::new();
        let a = store.create("person:mark", document("Trip")).await.unwrap();
        let kept = store
            .add_image("person:mark", a.id, picture("The Grand Palace"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kept.reference(), format!("niles-image:{}", kept.id));
        assert_eq!(
            (kept.width, kept.height, kept.mime.as_str()),
            (8, 6, "image/jpeg")
        );
        let listed = store.images("person:mark", a.id).await.unwrap();
        assert_eq!(listed, vec![kept.clone()]);
        let served = store
            .image("person:mark", a.id, kept.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(served.bytes.len(), kept.size);
        assert!(
            store
                .remove_image("person:mark", a.id, kept.id)
                .await
                .unwrap()
        );
        assert!(
            !store
                .remove_image("person:mark", a.id, kept.id)
                .await
                .unwrap()
        );
        assert!(store.images("person:mark", a.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_picture_is_never_another_persons_or_another_artifacts() {
        let store = MemoryArtifacts::new();
        let his = store.create("person:mark", document("His")).await.unwrap();
        let other = store
            .create("person:mark", document("Other"))
            .await
            .unwrap();
        let kept = store
            .add_image("person:mark", his.id, picture("x"))
            .await
            .unwrap()
            .unwrap();
        let her = "person:majse";
        assert!(
            store
                .add_image(her, his.id, picture("y"))
                .await
                .unwrap()
                .is_none()
        );
        assert!(store.images(her, his.id).await.unwrap().is_empty());
        assert!(store.image(her, his.id, kept.id).await.unwrap().is_none());
        assert!(!store.remove_image(her, his.id, kept.id).await.unwrap());
        assert!(
            store
                .image("person:mark", other.id, kept.id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !store
                .remove_image("person:mark", other.id, kept.id)
                .await
                .unwrap()
        );
        assert_eq!(store.images("person:mark", his.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_artifact_keeps_so_many_pictures_and_they_go_with_it() {
        let store = MemoryArtifacts::new();
        let a = store.create("person:mark", document("Trip")).await.unwrap();
        for i in 0..crate::images::MAX_PER_ARTIFACT {
            store
                .add_image("person:mark", a.id, picture(&format!("{i}")))
                .await
                .unwrap()
                .unwrap();
        }
        let e = store
            .add_image("person:mark", a.id, picture("one more"))
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("artifact_remove_image"), "{e}");
        assert!(store.delete("person:mark", a.id).await.unwrap());
        assert!(
            store.lock().images.is_empty(),
            "deleting takes its pictures"
        );
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
