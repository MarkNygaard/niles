//! What Niles told each person, kept for their Notifications page: every
//! push it sends them, and the notices it keeps without pushing first.
//! Postgres in the house (niles-db), in memory for tests.
//!
//! Kept whether or not the person has a device to push to, so the page
//! has the history either way. A notice with the tag of one kept before
//! replaces it, the way a push with that tag replaces the one on the
//! lock screen: it comes back as new, unread, with a new id, so ids go up
//! with time and "read up to this one" means what it says.

use crate::error::Result;
use crate::send::Message;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::Mutex;

/// How many one person keeps. Older ones go as new ones come.
pub const KEPT_PER_PERSON: i64 = 200;

/// Where a push that is not about an artifact opens the app.
pub const NOTIFICATIONS_URL: &str = "/#/notifications";

/// What a notification is about, which the page draws its icon by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// A job on an artifact finished or failed, or its page is ready.
    Artifact,
    /// Someone shared an artifact with the person.
    Shared,
    /// The coach changed a training plan after a run.
    PlanChanged,
    /// A workout from Strava was counted on a training plan.
    Workout,
    /// Something waits for the person's yes.
    Approval,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Artifact => "artifact",
            Kind::Shared => "shared",
            Kind::PlanChanged => "plan_changed",
            Kind::Workout => "workout",
            Kind::Approval => "approval",
        }
    }

    /// Whether its push opens the thing itself. One about an artifact
    /// or an approval opens it; the others open the Notifications page.
    fn opens_its_own_link(self) -> bool {
        matches!(
            self,
            Kind::Artifact | Kind::Shared | Kind::PlanChanged | Kind::Approval
        )
    }
}

/// A notification to keep and push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    /// Where the item on the page links.
    pub url: String,
    /// A newer one with the same tag replaces this one.
    pub tag: String,
}

impl Notice {
    /// The push for it: the same words, opening the artifact or the
    /// approval for one about either, and the Notifications page for
    /// the rest.
    pub fn push(&self) -> Message {
        Message {
            title: self.title.clone(),
            body: self.body.clone(),
            url: if self.kind.opens_its_own_link() {
                self.url.clone()
            } else {
                NOTIFICATIONS_URL.into()
            },
            tag: self.tag.clone(),
            approval: None,
            quiet: false,
        }
    }
}

/// One kept, as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notification {
    pub id: i64,
    /// [`Kind::as_str`]; a string, so a row an older Niles wrote still
    /// reads.
    pub kind: String,
    pub title: String,
    pub body: String,
    pub url: String,
    #[serde(skip)]
    pub tag: String,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait Inbox: Send + Sync {
    /// Kept for `person`, replacing one with the same tag. Only the
    /// newest [`KEPT_PER_PERSON`] stay.
    async fn add(&self, person: &str, notice: &Notice) -> Result<Notification>;
    /// `person`'s newest first: at most `limit`, all before the one
    /// `before` when it is given. Never anybody else's.
    async fn page(
        &self,
        person: &str,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<Notification>>;
    /// How many of `person`'s are unread.
    async fn unread(&self, person: &str) -> Result<i64>;
    /// `person`'s unread ones read now: those up to `up_to`, or all.
    /// Answers how many.
    async fn read(&self, person: &str, up_to: Option<i64>) -> Result<u64>;
}

#[derive(Default)]
struct Kept {
    next: i64,
    rows: Vec<(String, Notification)>,
}

/// In memory, for tests.
#[derive(Default)]
pub struct MemoryInbox {
    kept: Mutex<Kept>,
}

impl MemoryInbox {
    fn kept(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl Inbox for MemoryInbox {
    async fn add(&self, person: &str, notice: &Notice) -> Result<Notification> {
        let mut kept = self.kept();
        kept.next += 1;
        let row = Notification {
            id: kept.next,
            kind: notice.kind.as_str().into(),
            title: notice.title.clone(),
            body: notice.body.clone(),
            url: notice.url.clone(),
            tag: notice.tag.clone(),
            created_at: Utc::now(),
            read_at: None,
        };
        kept.rows
            .retain(|(p, n)| !(p == person && n.tag == notice.tag));
        kept.rows.push((person.into(), row.clone()));
        let theirs = kept.rows.iter().filter(|(p, _)| p == person).count();
        let over = theirs.saturating_sub(KEPT_PER_PERSON as usize);
        let mut dropped = 0;
        kept.rows.retain(|(p, _)| {
            if p == person && dropped < over {
                dropped += 1;
                return false;
            }
            true
        });
        Ok(row)
    }

    async fn page(
        &self,
        person: &str,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<Notification>> {
        Ok(self
            .kept()
            .rows
            .iter()
            .rev()
            .filter(|(p, n)| p == person && before.is_none_or(|b| n.id < b))
            .take(usize::try_from(limit).unwrap_or(0))
            .map(|(_, n)| n.clone())
            .collect())
    }

    async fn unread(&self, person: &str) -> Result<i64> {
        Ok(self
            .kept()
            .rows
            .iter()
            .filter(|(p, n)| p == person && n.read_at.is_none())
            .count() as i64)
    }

    async fn read(&self, person: &str, up_to: Option<i64>) -> Result<u64> {
        let now = Utc::now();
        let mut marked = 0;
        for (p, n) in self.kept().rows.iter_mut() {
            if p == person && n.read_at.is_none() && up_to.is_none_or(|u| n.id <= u) {
                n.read_at = Some(now);
                marked += 1;
            }
        }
        Ok(marked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(kind: Kind, tag: &str) -> Notice {
        Notice {
            kind,
            title: "Hellerup plan".into(),
            body: "Your 6.2 km run today was counted as the easy run.".into(),
            url: "/#/chat?artifact=1".into(),
            tag: tag.into(),
        }
    }

    #[test]
    fn a_push_about_an_artifact_opens_it_and_any_other_opens_the_page() {
        let ready = notice(Kind::Artifact, "artifact-1").push();
        assert_eq!(ready.url, "/#/chat?artifact=1");
        assert_eq!(ready.title, "Hellerup plan");
        assert_eq!(ready.tag, "artifact-1");
        assert_eq!(notice(Kind::Workout, "x").push().url, NOTIFICATIONS_URL);
        for kind in [Kind::Shared, Kind::PlanChanged, Kind::Approval] {
            assert_eq!(notice(kind, "x").push().url, "/#/chat?artifact=1");
        }
    }

    #[test]
    fn the_tag_is_never_shown() {
        let shown = serde_json::to_value(Notification {
            id: 1,
            kind: "workout".into(),
            title: "t".into(),
            body: "b".into(),
            url: "/".into(),
            tag: "workout-99".into(),
            created_at: Utc::now(),
            read_at: None,
        })
        .unwrap();
        assert!(shown.get("tag").is_none(), "{shown}");
        assert_eq!(shown["kind"], "workout");
    }

    #[tokio::test]
    async fn each_person_has_their_own_newest_first() {
        let inbox = MemoryInbox::default();
        let first = inbox
            .add("person:mark", &notice(Kind::Artifact, "a"))
            .await
            .unwrap();
        inbox
            .add("person:majse", &notice(Kind::Artifact, "a"))
            .await
            .unwrap();
        let second = inbox
            .add("person:mark", &notice(Kind::Workout, "b"))
            .await
            .unwrap();
        let mine = inbox.page("person:mark", None, 50).await.unwrap();
        assert_eq!(
            mine.iter().map(|n| n.id).collect::<Vec<_>>(),
            [second.id, first.id]
        );
        let older = inbox
            .page("person:mark", Some(second.id), 50)
            .await
            .unwrap();
        assert_eq!(older.len(), 1);
        assert_eq!(inbox.unread("person:majse").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn the_same_tag_replaces_and_comes_back_unread_and_newest() {
        let inbox = MemoryInbox::default();
        let old = inbox
            .add("person:mark", &notice(Kind::Artifact, "artifact-1"))
            .await
            .unwrap();
        inbox
            .add("person:mark", &notice(Kind::Workout, "workout-9"))
            .await
            .unwrap();
        inbox.read("person:mark", None).await.unwrap();
        let mut again = notice(Kind::Artifact, "artifact-1");
        again.body = "The page for your Hellerup plan is ready.".into();
        let new = inbox.add("person:mark", &again).await.unwrap();
        assert!(new.id > old.id);
        let mine = inbox.page("person:mark", None, 50).await.unwrap();
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].body, again.body);
        assert!(mine[0].read_at.is_none());
        assert_eq!(inbox.unread("person:mark").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn only_the_newest_are_kept() {
        let inbox = MemoryInbox::default();
        for n in 0..KEPT_PER_PERSON + 5 {
            inbox
                .add("person:mark", &notice(Kind::Workout, &format!("w{n}")))
                .await
                .unwrap();
        }
        inbox
            .add("person:majse", &notice(Kind::Workout, "w0"))
            .await
            .unwrap();
        let mine = inbox.page("person:mark", None, 1_000).await.unwrap();
        assert_eq!(mine.len() as i64, KEPT_PER_PERSON);
        assert_eq!(mine.last().unwrap().tag, "w5");
        assert_eq!(inbox.page("person:majse", None, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reading_marks_up_to_an_id_and_only_ones_own() {
        let inbox = MemoryInbox::default();
        let first = inbox
            .add("person:mark", &notice(Kind::Artifact, "a"))
            .await
            .unwrap();
        inbox
            .add("person:mark", &notice(Kind::Artifact, "b"))
            .await
            .unwrap();
        inbox
            .add("person:majse", &notice(Kind::Artifact, "a"))
            .await
            .unwrap();
        assert_eq!(inbox.read("person:mark", Some(first.id)).await.unwrap(), 1);
        assert_eq!(inbox.unread("person:mark").await.unwrap(), 1);
        assert_eq!(inbox.read("person:mark", None).await.unwrap(), 1);
        assert_eq!(inbox.read("person:mark", None).await.unwrap(), 0);
        assert_eq!(inbox.unread("person:majse").await.unwrap(), 1);
    }
}
