//! What changed between this build and the newest published one:
//! GitHub's compare API, asked without signing in.
//!
//! The anonymous limit is 60 requests an hour per address, so compare
//! runs only when the newest commit differs from the running one, and
//! the caller ([`crate::Updates`]) asks at most once per pair of
//! commits and backs off after a failure.
//!
//! Every link here is built from the repo constant and a validated sha
//! or PR number, never taken from GitHub's response.

use crate::build_info::Commit;
use crate::error::{Error, Result};
use crate::http;
use chrono::{DateTime, Utc};
use serde::Serialize;
use url::Url;

/// Where Niles's source lives; every link the app shows starts here.
pub const REPO_URL: &str = "https://github.com/MarkNygaard/niles";
/// GitHub's API, or a stand-in in tests.
pub(crate) const API: &str = "https://api.github.com";
const OWNER_REPO: [&str; 2] = ["MarkNygaard", "niles"];
/// How every error names the API in its reason.
const WHAT: &str = "GitHub's API";
/// The compare body carries file patches on its first page, so it is
/// far bigger than the commits it lists.
const MAX_COMPARE: usize = 16 * 1024 * 1024;
/// A commit subject, at most: it is a title, shown in a list.
const MAX_SUBJECT: usize = 200;

/// One commit between the running build and the newest, as What's new
/// shows it. `development` is squash-merged, so the subject is a pull
/// request title.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    pub sha: Commit,
    pub subject: String,
    /// The pull request a squash merge's `(#123)` names, when it does.
    pub pr: Option<u32>,
    /// The PR link when `pr` is known, else the commit's.
    pub url: String,
    pub at: Option<DateTime<Utc>>,
}

/// The comparison itself: which side is ahead, and the commits between,
/// newest first, merge commits left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Changes {
    pub ahead_by: u32,
    pub behind_by: u32,
    pub commits: Vec<Change>,
    /// GitHub lists at most 250 commits; a longer run is named as cut.
    pub truncated: bool,
}

/// GitHub's REST API, without a token. No `Default`: building the
/// client can fail, and an `expect` there would panic `serve`.
#[derive(Clone)]
pub struct GitHub {
    client: reqwest::Client,
    api: Url,
}

impl GitHub {
    /// The API at `base`: GitHub's own address, or a stand-in in tests.
    pub fn at(base: &str) -> Result<Self> {
        let api = Url::parse(base).map_err(|e| Error::Malformed {
            what: WHAT,
            reason: format!("{base}: {e}"),
        })?;
        Ok(Self {
            client: http::client()?,
            api,
        })
    }

    /// What commits `to` is ahead of `from` by. A 404 or 422 means one
    /// of the two is not on GitHub — a fork's commit, say — and what
    /// changed can't be listed, which the caller treats as "fall back
    /// to the dates", not as an error.
    pub async fn compare(&self, from: &Commit, to: &Commit) -> Result<Changes> {
        let response = self
            .client
            .get(self.compare_url(from, to))
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
            .send()
            .await
            .map_err(|e| http::send_error(WHAT, &e))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND
            || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY
        {
            return Err(Error::UnknownCommit);
        }
        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            if exhausted(&response) {
                return Err(Error::RateLimited {
                    what: WHAT,
                    until: resets_at(&response),
                });
            }
            return Err(Error::Status {
                what: WHAT,
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            return Err(Error::Status {
                what: WHAT,
                status: status.as_u16(),
            });
        }
        let bytes = http::read_capped(response, MAX_COMPARE, WHAT).await?;
        let raw: RawCompare = serde_json::from_slice(&bytes).map_err(|e| Error::Malformed {
            what: WHAT,
            reason: format!("its comparison doesn't read: {e}"),
        })?;
        Ok(read_changes(&raw))
    }

    fn compare_url(&self, from: &Commit, to: &Commit) -> Url {
        let mut url = self.api.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            // `from...to` is one segment: the three dots are a range,
            // not a path separator.
            path.pop_if_empty()
                .extend(["repos", OWNER_REPO[0], OWNER_REPO[1], "compare"]);
            path.push(&format!("{from}...{to}"));
        }
        url
    }
}

/// The comparison's page, ignoring everything but what What's new
/// shows (`files` among the rest).
#[derive(serde::Deserialize)]
struct RawCompare {
    #[serde(default)]
    ahead_by: u32,
    #[serde(default)]
    behind_by: u32,
    #[serde(default)]
    total_commits: usize,
    #[serde(default)]
    commits: Vec<RawCommit>,
}

#[derive(serde::Deserialize)]
struct RawCommit {
    #[serde(default)]
    sha: String,
    #[serde(default)]
    commit: RawMessage,
    #[serde(default)]
    parents: Vec<serde::de::IgnoredAny>,
}

#[derive(serde::Deserialize, Default)]
struct RawMessage {
    #[serde(default)]
    message: String,
    #[serde(default)]
    committer: Option<RawCommitter>,
}

#[derive(serde::Deserialize)]
struct RawCommitter {
    #[serde(default)]
    date: Option<String>,
}

/// A raw page as What's new shows it: merge commits dropped, subjects
/// cleaned and capped, links built from the repo, newest first.
fn read_changes(raw: &RawCompare) -> Changes {
    let mut commits: Vec<Change> = raw
        .commits
        .iter()
        .filter(|c| c.parents.len() <= 1)
        .filter_map(|c| {
            let sha = Commit::parse(&c.sha)?;
            let subject = subject_of(&c.commit.message);
            if subject.is_empty() {
                return None;
            }
            let pr = pr_number(&subject);
            let url = match pr {
                Some(n) => format!("{REPO_URL}/pull/{n}"),
                None => format!("{REPO_URL}/commit/{sha}"),
            };
            let at = c
                .commit
                .committer
                .as_ref()
                .and_then(|c| c.date.as_ref())
                .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
                .map(|d| d.with_timezone(&Utc));
            Some(Change {
                sha,
                subject,
                pr,
                url,
                at,
            })
        })
        .collect();
    // GitHub lists oldest first; the page shows newest first.
    commits.reverse();
    Changes {
        ahead_by: raw.ahead_by,
        behind_by: raw.behind_by,
        commits,
        truncated: raw.total_commits > raw.commits.len(),
    }
}

/// The first line of `message`: control characters removed (a subject
/// is shown as text in a list), trimmed, capped char-safe.
fn subject_of(message: &str) -> String {
    let first = message.lines().next().unwrap_or("");
    let cleaned: String = first.chars().filter(|c| !c.is_control()).collect();
    cleaned
        .trim()
        .chars()
        .take(MAX_SUBJECT)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The trailing `(#123)` of a squash-merged pull request's subject.
fn pr_number(subject: &str) -> Option<u32> {
    let inner = subject.trim_end().strip_suffix(')')?;
    let at = inner.rfind("(#")?;
    inner[at + 2..].parse().ok()
}

/// Where the whole comparison is read in a browser.
pub fn compare_url(from: &Commit, to: &Commit) -> String {
    format!("{REPO_URL}/compare/{from}...{to}")
}

/// Whether the refusal is the anonymous rate limit: the remaining
/// header reads 0.
fn exhausted(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        == Some("0")
}

/// When the rate limit lifts, as the reset header's epoch seconds.
fn resets_at(response: &reqwest::Response) -> Option<DateTime<Utc>> {
    response
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|secs| DateTime::from_timestamp(secs, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Three commits oldest first, the middle one a merge. Only the
    /// squash-merged PR titles survive, newest first.
    fn compare_page() -> serde_json::Value {
        let commit = |sha: &str, message: &str, parents: usize| {
            serde_json::json!({
                "sha": sha,
                "commit": {
                    "message": message,
                    "committer": {"date": "2026-11-04T10:00:00Z"}
                },
                "parents": (0..parents).map(|_| serde_json::json!({})).collect::<Vec<_>>(),
            })
        };
        serde_json::json!({
            "ahead_by": 2,
            "behind_by": 0,
            "total_commits": 3,
            "commits": [
                commit("1111111111111111111111111111111111111111", "feat: first (#510)", 1),
                commit("2222222222222222222222222222222222222222", "Merge pull request #511 from MarkNygaard/development", 2),
                commit("3333333333333333333333333333333333333333", "fix: second (#512)", 1),
            ]
        })
    }

    async fn serving(server: &MockServer, body: serde_json::Value, status: u16) -> GitHub {
        Mock::given(method("GET"))
            .and(path("/repos/MarkNygaard/niles/compare/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(server)
            .await;
        GitHub::at(&server.uri()).expect("the stand-in parses")
    }

    fn from() -> Commit {
        Commit::parse(&"a".repeat(40)).expect("40 hex digits")
    }

    fn to() -> Commit {
        Commit::parse(&"b".repeat(40)).expect("40 hex digits")
    }

    #[tokio::test]
    async fn ahead_lists_pull_requests_newest_first() {
        let server = MockServer::start().await;
        let github = serving(&server, compare_page(), 200).await;
        let changes = github
            .compare(&from(), &to())
            .await
            .expect("the page reads");
        assert_eq!(changes.ahead_by, 2);
        assert_eq!(changes.behind_by, 0);
        assert_eq!(changes.commits.len(), 2, "the merge is left out");
        assert!(!changes.truncated);
        let first = &changes.commits[0];
        assert_eq!(first.subject, "fix: second (#512)");
        assert_eq!(first.pr, Some(512));
        assert_eq!(first.url, "https://github.com/MarkNygaard/niles/pull/512");
        assert_eq!(
            first.sha.as_str(),
            "3333333333333333333333333333333333333333"
        );
        assert_eq!(
            first.at.map(|at| at.to_rfc3339()),
            Some("2026-11-04T10:00:00+00:00".to_string())
        );
        let second = &changes.commits[1];
        assert_eq!(second.subject, "feat: first (#510)");
        assert_eq!(second.pr, Some(510));
    }

    #[tokio::test]
    async fn a_page_shorter_than_total_is_truncated() {
        let server = MockServer::start().await;
        let mut page = compare_page();
        page["total_commits"] = serde_json::json!(40);
        let github = serving(&server, page, 200).await;
        let changes = github
            .compare(&from(), &to())
            .await
            .expect("the page reads");
        assert!(changes.truncated, "250 is the most GitHub lists");
    }

    #[tokio::test]
    async fn an_unknown_commit_cannot_be_listed() {
        let server = MockServer::start().await;
        let github = serving(&server, serde_json::json!({}), 404).await;
        let why = github.compare(&from(), &to()).await.unwrap_err();
        assert!(
            matches!(why, Error::UnknownCommit),
            "404 is an unknown commit: {why}"
        );
    }

    #[tokio::test]
    async fn the_rate_limit_says_until_when() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/MarkNygaard/niles/compare/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("x-ratelimit-reset", "1762380000"),
            )
            .mount(&server)
            .await;
        let github = GitHub::at(&server.uri()).expect("the stand-in parses");
        let why = github.compare(&from(), &to()).await.unwrap_err();
        let text = why.to_string();
        assert!(
            matches!(why, Error::RateLimited { until: Some(_), .. }),
            "the reset is read: {text}"
        );
        assert!(text.contains(" UTC"), "the message says until when: {text}");
    }

    #[test]
    fn long_subjects_are_capped_char_safe() {
        let long = "é".repeat(MAX_SUBJECT + 10);
        let page = serde_json::json!({
            "ahead_by": 1,
            "behind_by": 0,
            "total_commits": 1,
            "commits": [{
                "sha": "c".repeat(40),
                "commit": {"message": format!("{long}\n\nbody")},
                "parents": [{}],
            }],
        });
        let raw: RawCompare = serde_json::from_value(page).expect("the page reads");
        let changes = read_changes(&raw);
        assert_eq!(changes.commits[0].subject.chars().count(), MAX_SUBJECT);
    }

    #[test]
    fn pr_number_reads_the_trailing_marker() {
        assert_eq!(pr_number("fix: x (#12)"), Some(12));
        assert_eq!(pr_number("fix: x (#abc)"), None);
        assert_eq!(pr_number("Revert \"feat: y (#3)\" (#4)"), Some(4));
        assert_eq!(pr_number("chore: release 0.7.0 (#530)"), Some(530));
        assert_eq!(pr_number("no marker here"), None);
        assert_eq!(pr_number("x (#12) "), Some(12));
        assert_eq!(pr_number("fix: x (#12) extra"), None);
    }

    #[test]
    fn the_compare_link_names_both_commits() {
        assert_eq!(
            compare_url(&from(), &to()),
            "https://github.com/MarkNygaard/niles/compare/\
             aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }
}
