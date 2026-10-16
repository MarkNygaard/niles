use crate::kinds::Preview;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Whether an artifact can be read, is being written, or was not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ready,
    Working,
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Working => "working",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ready" => Some(Self::Ready),
            "working" => Some(Self::Working),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Who wrote a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    /// The person it belongs to, from the app: a ticked checklist item,
    /// a restore.
    Person,
    Claude,
    Chatgpt,
    Niles,
}

impl Author {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Claude => "claude",
            Self::Chatgpt => "chatgpt",
            Self::Niles => "niles",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "person" => Some(Self::Person),
            "claude" => Some(Self::Claude),
            "chatgpt" => Some(Self::Chatgpt),
            "niles" => Some(Self::Niles),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Artifact {
    pub id: i64,
    /// `person:{slug}` or `email:{address}`. Never sent to the app.
    #[serde(skip)]
    pub person: String,
    pub kind: String,
    pub title: String,
    /// One line, under the title.
    pub summary: String,
    pub pinned: bool,
    pub archived: bool,
    /// Whether the person is told when a job for it ends.
    pub notify: bool,
    pub status: Status,
    /// What a job is doing, while it does it.
    pub status_text: Option<String>,
    /// Why a job did not finish.
    pub error: Option<String>,
    #[serde(skip)]
    pub job_id: Option<String>,
    /// What a job was asked for, kept so it can be asked again.
    #[serde(skip)]
    pub brief: Option<String>,
    /// The chat a job was started from, where its card goes.
    #[serde(skip)]
    pub conversation_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The version shown; 0 before there is one.
    pub current_revision: i32,
}

/// One version of an artifact's content.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Revision {
    pub n: i32,
    pub content: Value,
    pub reason: String,
    pub author: Author,
    pub at: DateTime<Utc>,
}

/// A version as the list of versions shows it, without its content.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RevisionMeta {
    pub n: i32,
    pub reason: String,
    pub author: Author,
    pub at: DateTime<Utc>,
}

impl From<&Revision> for RevisionMeta {
    fn from(r: &Revision) -> Self {
        Self {
            n: r.n,
            reason: r.reason.clone(),
            author: r.author,
            at: r.at,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewArtifact {
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub notify: bool,
    pub status: Status,
    pub status_text: Option<String>,
    pub job_id: Option<String>,
    pub brief: Option<String>,
    pub conversation_id: Option<i64>,
    /// The first version, when it is made at once rather than by a job.
    pub first: Option<NewRevision>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewRevision {
    /// Already checked by its kind's validator.
    pub content: Value,
    pub reason: String,
    pub author: Author,
    /// A new summary to go with it, if it changes.
    pub summary: Option<String>,
}

/// What the app changes about an artifact other than its content. Each
/// `None` leaves that as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    pub title: Option<String>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
    pub notify: Option<bool>,
}

/// Where a job has got to. The status text, error and job id are set
/// as given, so going back to `ready` clears what `working` said.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusChange {
    pub status: Status,
    pub status_text: Option<String>,
    pub error: Option<String>,
    pub job_id: Option<String>,
}

/// Which artifacts a list holds. Each `None` is either.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Filter {
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
}

impl Filter {
    pub fn admits(&self, a: &Artifact) -> bool {
        self.pinned.is_none_or(|p| a.pinned == p) && self.archived.is_none_or(|x| a.archived == x)
    }
}

/// An artifact as a chat turn carries it: enough for a card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: Status,
    #[serde(default)]
    pub preview: Preview,
}

impl ArtifactRef {
    /// The card for `artifact`, with the preview of `content` when there
    /// is a version to show.
    pub fn of(artifact: &Artifact, content: Option<&Value>) -> Self {
        Self {
            id: artifact.id,
            kind: artifact.kind.clone(),
            title: artifact.title.clone(),
            summary: artifact.summary.clone(),
            status: artifact.status,
            preview: content
                .map(|c| crate::kinds::preview(&artifact.kind, c))
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_authors_read_back_as_written() {
        for s in [Status::Ready, Status::Working, Status::Failed] {
            assert_eq!(Status::parse(s.as_str()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
        for a in [
            Author::Person,
            Author::Claude,
            Author::Chatgpt,
            Author::Niles,
        ] {
            assert_eq!(Author::parse(a.as_str()), Some(a));
            assert_eq!(serde_json::to_value(a).unwrap(), a.as_str());
        }
        assert_eq!(Status::parse("done"), None);
    }

    #[test]
    fn whose_it_is_never_goes_to_the_app() {
        let now = Utc::now();
        let a = Artifact {
            id: 1,
            person: "person:mark".into(),
            kind: "document".into(),
            title: "Packing list".into(),
            summary: "For the cabin".into(),
            pinned: false,
            archived: false,
            notify: true,
            status: Status::Ready,
            status_text: None,
            error: None,
            job_id: Some("j".into()),
            brief: Some("what to pack".into()),
            conversation_id: Some(3),
            created_at: now,
            updated_at: now,
            current_revision: 1,
        };
        let v = serde_json::to_value(&a).unwrap();
        assert!(v.get("person").is_none());
        assert!(v.get("brief").is_none());
        assert_eq!(v["title"], "Packing list");
    }
}
