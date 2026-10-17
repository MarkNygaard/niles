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

/// What wrote a version. Which person, when it was one or one's
/// assistant, is the version's `by`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    /// A person, from the app: a ticked checklist item, a restore. Its
    /// owner, or someone it is shared with to edit.
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
    /// In the list of whoever reads it: the owner's own pin, or, for an
    /// artifact shared with someone, theirs.
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
    /// The first lines of the version shown, kept with the row so a
    /// list needs no content.
    pub preview: Preview,
    /// The page the version shown is drawn with, by its id; `None` for
    /// the kind's own view.
    pub page: Option<i64>,
    /// Whose change the version shown is, by key: the person who made
    /// it, or whose assistant did. `None` for Niles's own bookkeeping,
    /// and for a version kept before anyone else could change it.
    #[serde(skip)]
    pub changed_by: Option<String>,
    /// Everyone it is shared with, by key, in the order they were
    /// shared with: who else hears that it changed.
    #[serde(skip)]
    pub shared_with: Vec<String>,
}

/// One version of an artifact's content.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Revision {
    pub n: i32,
    pub content: Value,
    /// The page it was drawn with then, by its id.
    pub page: Option<i64>,
    pub reason: String,
    pub author: Author,
    /// Whose change it was, by key; see [`Artifact::changed_by`].
    #[serde(skip)]
    pub by: Option<String>,
    pub at: DateTime<Utc>,
}

/// A version as the list of versions shows it, without its content.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RevisionMeta {
    pub n: i32,
    pub page: Option<i64>,
    pub reason: String,
    pub author: Author,
    #[serde(skip)]
    pub by: Option<String>,
    pub at: DateTime<Utc>,
}

impl From<&Revision> for RevisionMeta {
    fn from(r: &Revision) -> Self {
        Self {
            n: r.n,
            page: r.page,
            reason: r.reason.clone(),
            author: r.author,
            by: r.by.clone(),
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
    /// Whose change it is, by key: the person, or whose assistant made
    /// it; `None` for Niles's own.
    pub by: Option<String>,
    /// A new summary to go with it, if it changes.
    pub summary: Option<String>,
    /// What becomes of the page. Every change to the data keeps it.
    pub page: PageChange,
}

/// What a new version does with the artifact's page. A page is kept
/// once, however many versions draw with it: Niles marks a run on a
/// plan many times a month, and a copy of a 200 KB page in each would
/// be most of what is kept.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum PageChange {
    /// The page the current version has, if any.
    #[default]
    Keep,
    /// A new page, already checked by [`crate::page::page`].
    New(String),
    /// A page of this artifact's that is kept already: a restored
    /// version's.
    Use(i64),
    /// None: the kind's own view.
    Remove,
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

/// Long enough for "Thailand, two weeks in March: Bangkok and the
/// islands", short enough for one line of a list.
pub const MAX_TITLE: usize = 120;
/// One line under the title.
pub const MAX_SUMMARY: usize = 240;

/// A title as it is kept: on one line, not empty, not too long.
pub fn title(raw: &str) -> crate::Result<String> {
    let t = one_line(raw);
    if t.is_empty() {
        return Err(crate::Error::Invalid {
            kind: "title",
            reason: "it is empty".into(),
        });
    }
    if t.chars().count() > MAX_TITLE {
        return Err(crate::Error::Invalid {
            kind: "title",
            reason: format!("it is longer than {MAX_TITLE} characters"),
        });
    }
    Ok(t)
}

/// A summary as it is kept: one line, cut if it runs long rather than
/// refused, since it only describes what is already saved.
pub fn summary(raw: &str) -> String {
    let s = one_line(raw);
    if s.chars().count() <= MAX_SUMMARY {
        return s;
    }
    let cut: String = s.chars().take(MAX_SUMMARY - 1).collect();
    format!("{}…", cut.trim_end())
}

fn one_line(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
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
    pub fn of(artifact: &Artifact) -> Self {
        Self {
            id: artifact.id,
            kind: artifact.kind.clone(),
            title: artifact.title.clone(),
            summary: artifact.summary.clone(),
            status: artifact.status,
            preview: artifact.preview.clone(),
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
    fn a_title_is_one_line_and_never_empty() {
        assert_eq!(
            title(
                "  Thailand
 travel plan "
            )
            .unwrap(),
            "Thailand travel plan"
        );
        assert!(title("   ").is_err());
        assert!(title(&"x".repeat(MAX_TITLE + 1)).is_err());
    }

    #[test]
    fn a_long_summary_is_cut_rather_than_refused() {
        let s = summary(&"word ".repeat(100));
        assert!(s.ends_with('…'));
        assert!(s.chars().count() <= MAX_SUMMARY);
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
            preview: Preview::default(),
            page: None,
            changed_by: Some("person:majse".into()),
            shared_with: vec!["person:majse".into()],
        };
        let v = serde_json::to_value(&a).unwrap();
        assert!(v.get("person").is_none());
        assert!(v.get("brief").is_none());
        assert!(v.get("changed_by").is_none());
        assert!(v.get("shared_with").is_none());
        assert_eq!(v["title"], "Packing list");
    }
}
