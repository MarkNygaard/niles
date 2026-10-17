//! Sharing an artifact with someone else in the household, to read or
//! to edit. See ARCHITECTURE.md § Artifacts (per person), Sharing.
//!
//! An artifact stays its owner's: shared, it is in the other person's
//! Saved list as well, and their assistant can read it, or change it
//! when it is shared to edit. What each may do is [`Access::allows`],
//! and everything that acts on an artifact for a person asks it through
//! [`open`]: the app's routes and the assistant's tools alike, so the
//! two cannot disagree about who may do what.

use crate::error::Result;
use crate::model::Artifact;
use crate::store::ArtifactStore;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How an artifact is shared with someone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// To read: it is in their list, and opens, and nothing more.
    View,
    /// To change as its owner does, short of what makes it theirs.
    Edit,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Edit => "edit",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "view" => Some(Self::View),
            "edit" => Some(Self::Edit),
            _ => None,
        }
    }
}

/// What a person has of an artifact they can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Owner,
    Edit,
    View,
}

impl From<Level> for Access {
    fn from(level: Level) -> Self {
        match level {
            Level::View => Self::View,
            Level::Edit => Self::Edit,
        }
    }
}

/// Everything done with an artifact, by what it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Open it: the version shown, its page, its pictures, who changed
    /// it last.
    Read,
    /// Pin it in one's own list, which nobody else's follows.
    Pin,
    /// Read the versions before the one shown. Not for someone it is
    /// shared with to read: what it was before it was shared is the
    /// owner's.
    History,
    /// Change it: its content (a tick, a mark, a page's save), its
    /// title, its page and its pictures; restore a version; try its job
    /// again, or make it a page.
    Edit,
    /// Whether its owner is told when a job for it ends.
    Notify,
    Archive,
    /// Share it, change how, or stop.
    Share,
    Delete,
}

impl Access {
    pub fn allows(self, op: Operation) -> bool {
        match op {
            Operation::Read | Operation::Pin => true,
            Operation::History | Operation::Edit => self != Self::View,
            Operation::Notify | Operation::Archive | Operation::Share | Operation::Delete => {
                self == Self::Owner
            }
        }
    }
}

/// Someone an artifact is shared with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    /// Their key, as their own things are kept under.
    pub person: String,
    pub level: Level,
    pub shared_at: DateTime<Utc>,
}

/// Why an artifact is not shared with its own owner: it is theirs.
pub fn with_the_owner() -> crate::Error {
    crate::Error::Invalid {
        kind: "share",
        reason: "it is theirs already".into(),
    }
}

/// An artifact as one person has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    /// With the pin of the person who opened it.
    pub artifact: Artifact,
    pub access: Access,
}

impl Opened {
    /// Whose it is, the key every write to it goes under.
    pub fn owner(&self) -> &str {
        &self.artifact.person
    }

    pub fn allows(&self, op: Operation) -> bool {
        self.access.allows(op)
    }
}

/// Artifact `id` as `person` has it: theirs, or shared with them. `None`
/// when it is neither, which is no different from there being none.
pub async fn open(store: &dyn ArtifactStore, person: &str, id: i64) -> Result<Option<Opened>> {
    if let Some(artifact) = store.get(person, id).await? {
        return Ok(Some(Opened {
            artifact,
            access: Access::Owner,
        }));
    }
    Ok(store
        .shared(person, id)
        .await?
        .map(|(artifact, level)| Opened {
            artifact,
            access: level.into(),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Author, Changes, Filter, NewArtifact, NewRevision, PageChange, Status};
    use crate::store::MemoryArtifacts;
    use serde_json::json;

    const MARK: &str = "person:mark";
    const MAJSE: &str = "person:majse";
    const ANNA: &str = "email:anna@example.com";

    const EVERY: [Operation; 8] = [
        Operation::Read,
        Operation::Pin,
        Operation::History,
        Operation::Edit,
        Operation::Notify,
        Operation::Archive,
        Operation::Share,
        Operation::Delete,
    ];

    #[test]
    fn each_access_allows_what_it_should_and_nothing_more() {
        let allowed = |access: Access| -> Vec<Operation> {
            EVERY.into_iter().filter(|op| access.allows(*op)).collect()
        };
        assert_eq!(allowed(Access::Owner), EVERY.to_vec());
        assert_eq!(
            allowed(Access::Edit),
            vec![
                Operation::Read,
                Operation::Pin,
                Operation::History,
                Operation::Edit
            ]
        );
        assert_eq!(allowed(Access::View), vec![Operation::Read, Operation::Pin]);
    }

    #[test]
    fn levels_read_back_as_written() {
        for level in [Level::View, Level::Edit] {
            assert_eq!(Level::parse(level.as_str()), Some(level));
            assert_eq!(serde_json::to_value(level).unwrap(), level.as_str());
        }
        assert_eq!(Level::parse("owner"), None);
        assert_eq!(serde_json::to_value(Access::Owner).unwrap(), "owner");
    }

    async fn trip(store: &MemoryArtifacts) -> Artifact {
        store
            .create(
                MARK,
                NewArtifact {
                    kind: "document".into(),
                    title: "Thailand trip".into(),
                    summary: String::new(),
                    notify: true,
                    status: Status::Ready,
                    status_text: None,
                    job_id: None,
                    brief: None,
                    conversation_id: None,
                    first: Some(NewRevision {
                        content: json!({ "markdown": "- [ ] passport" }),
                        reason: "First draft".into(),
                        author: Author::Claude,
                        by: Some(MARK.into()),
                        summary: None,
                        page: PageChange::Keep,
                    }),
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_owner_opens_it_as_owner_and_nobody_else_at_all() {
        let store = MemoryArtifacts::new();
        let a = trip(&store).await;
        let mine = open(&store, MARK, a.id).await.unwrap().unwrap();
        assert_eq!(mine.access, Access::Owner);
        assert_eq!(mine.owner(), MARK);
        assert!(open(&store, MAJSE, a.id).await.unwrap().is_none());
        assert!(open(&store, MARK, a.id + 1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn shared_it_opens_at_its_level_and_stops_when_unshared() {
        let store = MemoryArtifacts::new();
        let a = trip(&store).await;
        assert!(
            store
                .set_share(MARK, a.id, MAJSE, Some(Level::View))
                .await
                .unwrap()
        );
        assert!(
            store
                .set_share(MARK, a.id, ANNA, Some(Level::Edit))
                .await
                .unwrap()
        );
        let hers = open(&store, MAJSE, a.id).await.unwrap().unwrap();
        assert_eq!(hers.access, Access::View);
        assert_eq!(hers.owner(), MARK);
        assert_eq!(hers.artifact.shared_with, vec![MAJSE, ANNA]);
        assert_eq!(
            open(&store, ANNA, a.id).await.unwrap().unwrap().access,
            Access::Edit
        );

        store
            .set_share(MARK, a.id, MAJSE, Some(Level::Edit))
            .await
            .unwrap();
        assert_eq!(
            open(&store, MAJSE, a.id).await.unwrap().unwrap().access,
            Access::Edit
        );
        let shares = store.shares(MARK, a.id).await.unwrap();
        assert_eq!(shares.len(), 2, "changing the level is not a second share");

        store.set_share(MARK, a.id, MAJSE, None).await.unwrap();
        assert!(open(&store, MAJSE, a.id).await.unwrap().is_none());
        assert_eq!(
            store.get(MARK, a.id).await.unwrap().unwrap().shared_with,
            vec![ANNA]
        );
    }

    #[tokio::test]
    async fn only_the_owner_shares_and_never_with_themselves() {
        let store = MemoryArtifacts::new();
        let a = trip(&store).await;
        store
            .set_share(MARK, a.id, MAJSE, Some(Level::Edit))
            .await
            .unwrap();
        // An editor is not the owner: sharing it on is not theirs.
        assert!(
            !store
                .set_share(MAJSE, a.id, ANNA, Some(Level::View))
                .await
                .unwrap()
        );
        assert!(store.shares(MAJSE, a.id).await.unwrap().is_empty());
        assert!(
            store
                .set_share(MARK, a.id, MARK, Some(Level::View))
                .await
                .is_err()
        );
        assert!(open(&store, ANNA, a.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn shared_ones_are_listed_for_whoever_they_are_shared_with_by_their_own_pin() {
        let store = MemoryArtifacts::new();
        let a = trip(&store).await;
        store
            .change(
                MARK,
                a.id,
                Changes {
                    pinned: Some(true),
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        store
            .set_share(MARK, a.id, MAJSE, Some(Level::View))
            .await
            .unwrap();
        assert!(
            store
                .list(MAJSE, Filter::default())
                .await
                .unwrap()
                .is_empty(),
            "her own list is hers alone"
        );
        let shared = store.shared_with(MAJSE, Filter::default()).await.unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].1, Level::View);
        assert!(!shared[0].0.pinned, "his pin is not hers");

        assert!(store.pin_shared(MAJSE, a.id, true).await.unwrap());
        let pinned = Filter {
            pinned: Some(true),
            ..Filter::default()
        };
        assert_eq!(store.shared_with(MAJSE, pinned).await.unwrap().len(), 1);
        assert!(
            open(&store, MAJSE, a.id)
                .await
                .unwrap()
                .unwrap()
                .artifact
                .pinned
        );

        store
            .change(
                MARK,
                a.id,
                Changes {
                    pinned: Some(false),
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        assert!(
            open(&store, MAJSE, a.id)
                .await
                .unwrap()
                .unwrap()
                .artifact
                .pinned
        );
        assert!(!store.get(MARK, a.id).await.unwrap().unwrap().pinned);
        assert!(
            !store.pin_shared(ANNA, a.id, true).await.unwrap(),
            "nothing to pin for someone it is not shared with"
        );
    }

    #[tokio::test]
    async fn deleting_it_takes_its_shares_with_it() {
        let store = MemoryArtifacts::new();
        let a = trip(&store).await;
        store
            .set_share(MARK, a.id, MAJSE, Some(Level::Edit))
            .await
            .unwrap();
        assert!(store.delete(MARK, a.id).await.unwrap());
        assert!(open(&store, MAJSE, a.id).await.unwrap().is_none());
        assert!(
            store
                .shared_with(MAJSE, Filter::default())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_change_records_whose_it_was() {
        let store = MemoryArtifacts::new();
        let a = trip(&store).await;
        assert_eq!(a.changed_by.as_deref(), Some(MARK));
        let kept = store
            .revise(
                MARK,
                a.id,
                NewRevision {
                    content: json!({ "markdown": "- [x] passport" }),
                    reason: "Ticked passport".into(),
                    author: Author::Person,
                    by: Some(MAJSE.into()),
                    summary: None,
                    page: PageChange::Keep,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kept.changed_by.as_deref(), Some(MAJSE));
        let r = store.revision(MARK, a.id, 2).await.unwrap().unwrap();
        assert_eq!(r.by.as_deref(), Some(MAJSE));
        assert_eq!(
            store.revisions(MARK, a.id).await.unwrap()[0].by.as_deref(),
            Some(MAJSE)
        );
        let restored = crate::restore(&store, MARK, a.id, 1, ANNA)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.changed_by.as_deref(), Some(ANNA));
    }
}
