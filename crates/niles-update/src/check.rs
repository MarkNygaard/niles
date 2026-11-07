//! The update check: what is kept, and the loop that refreshes it.
//!
//! The answer lives in memory. [`Updates::status`] never touches the
//! network, so `GET /system/update` answers at once; [`Updates::check`]
//! checks now, one at a time ([`Updates::flight`]) and no more than
//! once in [`MIN_INTERVAL`], so a page opened twice, StrictMode's
//! double mount and a household of people can't hammer GitHub's
//! anonymous limit.
//!
//! A failure keeps the last answer and names itself beside it: offline
//! is "couldn't check", never an error.

use crate::build_info::{Build, Commit};
use crate::changes::{Change, Changes, GitHub};
use crate::error::Error;
use crate::registry::{Newest, Registry};
use chrono::{DateTime, Utc};
use niles_core::{Event, EventBus};
use serde::Serialize;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

/// When the background loop first checks: late enough not to slow the
/// start, far enough that a crash-looping pod doesn't hammer GHCR.
const FIRST_CHECK: Duration = Duration::from_secs(2 * 60);
/// Then how often.
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// After a failure, how soon it tries again.
const RETRY: Duration = Duration::from_secs(30 * 60);
/// An attempt younger than this is reused, not repeated.
const MIN_INTERVAL: Duration = Duration::from_secs(30);
/// A failed comparison is tried again no sooner than this.
const COMPARE_RETRY: Duration = Duration::from_secs(15 * 60);
/// How long the registry part of a check may take in all.
const REGISTRY_BUDGET: Duration = Duration::from_secs(30);
/// How long the comparison may take. Separate from the registry budget,
/// so a slow compare never loses a good registry answer.
const COMPARE_BUDGET: Duration = Duration::from_secs(20);
/// Where Kubernetes mounts the pod's own namespace.
const NAMESPACE_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// The headline of the answer.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// A build without a commit never checks.
    DevBuild,
    /// No successful check yet.
    NotChecked,
    /// Nothing newer is published.
    UpToDate,
    /// A newer build is published.
    UpdateAvailable,
}

/// A failed attempt, named in the reason.
#[derive(Debug, Clone, Serialize)]
pub struct Failed {
    pub at: DateTime<Utc>,
    pub reason: String,
}

/// What changed between the running build and the newest, newest first.
#[derive(Debug, Clone, Serialize)]
pub struct WhatsNew {
    pub commits: Vec<Change>,
    pub truncated: bool,
}

/// What the last attempt answered, kept for the API and the app.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub running: Build,
    pub state: State,
    /// The last successful check.
    pub checked_at: Option<DateTime<Utc>>,
    /// Set when the latest attempt failed; cleared by the next success.
    pub failed: Option<Failed>,
    pub newest: Option<Newest>,
    /// This build is ahead of `latest`.
    pub newer_than_published: bool,
    pub changes: Option<WhatsNew>,
    pub changes_failed: Option<String>,
    pub compare_url: Option<String>,
    /// The pod's namespace, for the command About Niles offers; absent
    /// outside Kubernetes.
    pub namespace: Option<String>,
}

/// The check itself: the build it runs as, the two clients, the bus it
/// announces on, and what it has found.
pub struct Updates {
    build: Build,
    registry: Registry,
    github: GitHub,
    bus: EventBus,
    namespace: Option<String>,
    min_interval: Duration,
    compare_retry: Duration,
    /// Never held across an `.await`.
    held: Mutex<Held>,
    /// One check at a time.
    flight: AsyncMutex<()>,
}

struct Found {
    checked_at: DateTime<Utc>,
    newest: Newest,
}

struct Compared {
    to: Commit,
    at: Instant,
    result: std::result::Result<Changes, String>,
}

#[derive(Default)]
struct Held {
    found: Option<Found>,
    failed: Option<Failed>,
    compared: Option<Compared>,
    last_attempt: Option<Instant>,
}

impl Updates {
    /// The check as `niles serve` runs it: the real registry and API,
    /// and the pod's namespace when there is one.
    pub fn new(build: Build, bus: EventBus) -> crate::Result<Self> {
        let registry = Registry::at(crate::registry::REGISTRY)?;
        let github = GitHub::at(crate::changes::API)?;
        let namespace = std::fs::read_to_string(NAMESPACE_FILE)
            .ok()
            .and_then(|t| namespace_from(&t));
        Ok(Self::assembled(build, bus, registry, github, namespace))
    }

    /// The check over stand-ins, with no namespace: tests now, phase 2's
    /// in-cluster client when it lands.
    pub fn at(build: Build, bus: EventBus, registry: Registry, github: GitHub) -> Self {
        Self::assembled(build, bus, registry, github, None)
    }

    fn assembled(
        build: Build,
        bus: EventBus,
        registry: Registry,
        github: GitHub,
        namespace: Option<String>,
    ) -> Self {
        Self {
            build,
            registry,
            github,
            bus,
            namespace,
            min_interval: MIN_INTERVAL,
            compare_retry: COMPARE_RETRY,
            held: Mutex::default(),
            flight: AsyncMutex::default(),
        }
    }

    /// Shorter windows, for a test that drives the throttle or the
    /// compare retry by hand.
    #[cfg(test)]
    fn with_intervals(mut self, min_interval: Duration, compare_retry: Duration) -> Self {
        self.min_interval = min_interval;
        self.compare_retry = compare_retry;
        self
    }

    pub fn build(&self) -> &Build {
        &self.build
    }

    /// The kept answer, as the API serves it. Never touches the
    /// network and never waits: the state is worked out from what the
    /// last check stored.
    pub fn status(&self) -> Status {
        let held = self.lock();
        let Verdict {
            state,
            newer_than_published,
        } = verdict(&self.build, held.found.as_ref(), held.compared.as_ref());
        let matching = held
            .found
            .as_ref()
            .zip(held.compared.as_ref())
            .filter(|(f, c)| c.to == f.newest.commit);
        let (changes, changes_failed) = if state == State::UpdateAvailable {
            match matching.map(|(_, c)| &c.result) {
                Some(Ok(c)) => (
                    Some(WhatsNew {
                        commits: c.commits.clone(),
                        truncated: c.truncated,
                    }),
                    None,
                ),
                Some(Err(reason)) => (None, Some(reason.clone())),
                None => (None, None),
            }
        } else {
            (None, None)
        };
        let compare_url = match (self.build.commit(), held.found.as_ref()) {
            (Some(running), Some(found)) if running != &found.newest.commit => {
                Some(crate::changes::compare_url(running, &found.newest.commit))
            }
            _ => None,
        };
        Status {
            running: self.build.clone(),
            state,
            checked_at: held.found.as_ref().map(|f| f.checked_at),
            failed: held.failed.clone(),
            newest: held.found.as_ref().map(|f| f.newest.clone()),
            newer_than_published,
            changes,
            changes_failed,
            compare_url,
            namespace: self.namespace.clone(),
        }
    }

    /// Check now, and answer with what is kept. One at a time, and a
    /// check within [`MIN_INTERVAL`] of the last attempt reuses its
    /// answer. Announces [`Event::UpdateChecked`] after every check that
    /// touched the network.
    pub async fn check(&self) -> Status {
        let Some(running) = self.build.commit().cloned() else {
            // A dev build says so and makes no request; see the crate
            // doc for why.
            return self.status();
        };
        let _flight = self.flight.lock().await;
        let (known, throttled) = {
            let held = self.lock();
            (
                held.found.as_ref().map(|f| f.newest.clone()),
                held.last_attempt
                    .is_some_and(|at| at.elapsed() < self.min_interval),
            )
        };
        if throttled {
            return self.status();
        }
        match timeout(REGISTRY_BUDGET, self.registry.newest(known.as_ref())).await {
            Ok(Ok(newest)) => self.store_found(&running, newest).await,
            Ok(Err(why)) => self.store_failed(why),
            Err(_) => self.store_failed(Error::Timeout {
                what: "GitHub's container registry",
                after_secs: REGISTRY_BUDGET.as_secs(),
            }),
        }
        self.bus.publish(Event::UpdateChecked);
        self.status()
    }

    /// The background loop: [`FIRST_CHECK`] after the start, then
    /// [`EVERY`], or [`RETRY`] after a failure. A dev build has no loop.
    pub fn spawn(self: std::sync::Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        if self.build.commit().is_none() {
            tracing::info!("[update] a dev build, so Niles doesn't check for a newer one");
            return None;
        }
        Some(tokio::spawn(async move {
            tokio::time::sleep(FIRST_CHECK).await;
            loop {
                let status = self.check().await;
                let wait = if status.failed.is_some() {
                    RETRY
                } else {
                    EVERY
                };
                tokio::time::sleep(wait).await;
            }
        }))
    }

    /// A registry answer: keep it, clear the failure, and compare when
    /// the newest commit differs from this build's and no comparison
    /// for it is fresh.
    async fn store_found(&self, running: &Commit, newest: Newest) {
        let compare_due = {
            let held = self.lock();
            match &held.compared {
                None => true,
                Some(c) => {
                    c.to != newest.commit
                        || (c.result.is_err() && c.at.elapsed() >= self.compare_retry)
                }
            }
        };
        {
            let mut held = self.lock();
            if held
                .found
                .as_ref()
                .is_none_or(|f| f.newest.commit != newest.commit)
            {
                tracing::info!(
                    "[update] the newest published Niles is {}, built {}; this one is {}",
                    newest.commit.short(),
                    newest
                        .created
                        .map(|c| c.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_else(|| "at an unknown time".to_string()),
                    running.short(),
                );
            }
            held.found = Some(Found {
                checked_at: Utc::now(),
                newest: newest.clone(),
            });
            held.failed = None;
            held.last_attempt = Some(Instant::now());
        }
        if newest.commit != *running && compare_due {
            let result =
                match timeout(COMPARE_BUDGET, self.github.compare(running, &newest.commit)).await {
                    Ok(result) => result.map_err(|e| e.to_string()),
                    Err(_) => Err(Error::Timeout {
                        what: "GitHub's API",
                        after_secs: COMPARE_BUDGET.as_secs(),
                    }
                    .to_string()),
                };
            if let Err(reason) = &result {
                tracing::info!("[update] couldn't list what changed: {reason}");
            }
            self.lock().compared = Some(Compared {
                to: newest.commit.clone(),
                at: Instant::now(),
                result,
            });
        }
    }

    /// A failed attempt: keep the last answer, name the failure. Said
    /// loudly after a run of successes, quietly while one failure
    /// follows another.
    fn store_failed(&self, error: Error) {
        let reason = error.to_string();
        let mut held = self.lock();
        if held.failed.is_none() {
            tracing::info!("[update] couldn't check for a newer Niles: {reason}");
        } else {
            tracing::debug!("[update] still can't check for a newer Niles: {reason}");
        }
        held.failed = Some(Failed {
            at: Utc::now(),
            reason,
        });
        held.last_attempt = Some(Instant::now());
    }

    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The state, from what is kept. A named struct, not a bool in a
/// tuple, because `true` at the call site wouldn't say which way.
struct Verdict {
    state: State,
    newer_than_published: bool,
}

fn verdict(build: &Build, found: Option<&Found>, compared: Option<&Compared>) -> Verdict {
    let up_to_date = Verdict {
        state: State::UpToDate,
        newer_than_published: false,
    };
    let Some(running) = build.commit() else {
        return Verdict {
            state: State::DevBuild,
            newer_than_published: false,
        };
    };
    let Some(found) = found else {
        return Verdict {
            state: State::NotChecked,
            newer_than_published: false,
        };
    };
    if found.newest.commit == *running {
        return up_to_date;
    }
    if let Some(compared) = compared
        && compared.to == found.newest.commit
    {
        match &compared.result {
            Ok(c) if c.ahead_by > 0 => {
                return Verdict {
                    state: State::UpdateAvailable,
                    newer_than_published: false,
                };
            }
            Ok(c) => {
                return Verdict {
                    state: State::UpToDate,
                    newer_than_published: c.behind_by > 0,
                };
            }
            Err(_) => {}
        }
    }
    match (found.newest.created, build.built_at()) {
        // No comparison (or a failed one): the later build date
        // decides.
        (Some(created), Some(built_at)) => match created.cmp(&built_at) {
            std::cmp::Ordering::Greater => Verdict {
                state: State::UpdateAvailable,
                newer_than_published: false,
            },
            std::cmp::Ordering::Less => Verdict {
                state: State::UpToDate,
                newer_than_published: true,
            },
            std::cmp::Ordering::Equal => up_to_date,
        },
        // A different published build is offered, and nothing says it
        // is older: that is an update.
        _ => Verdict {
            state: State::UpdateAvailable,
            newer_than_published: false,
        },
    }
}

/// A namespace as it may end up in a command shown for copying: an RFC
/// 1123 label, nothing else.
fn namespace_from(text: &str) -> Option<String> {
    let label = text.trim();
    let bytes = label.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= 63
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes.first().is_some_and(|b| *b != b'-')
        && bytes.last().is_some_and(|b| *b != b'-');
    valid.then(|| label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ImageDigest;
    use std::sync::Arc;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RUNNING: &str = "0123456789abcdef0123456789abcdef01234567";
    const NEWER: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const BUILT_AT: &str = "2026-11-01T00:00:00Z";
    const NEWER_CREATED: &str = "2026-11-05T09:30:00Z";

    fn sha(text: &str) -> String {
        use sha2::Digest;
        format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(text.as_bytes()))
        )
    }

    fn running_build() -> Build {
        Build::new("0.6.0", Some(RUNNING), Some(BUILT_AT))
    }

    fn dev_build() -> Build {
        Build::new("0.6.0", None, None)
    }

    fn digest_of(text: &str) -> ImageDigest {
        ImageDigest::parse(&sha(text)).expect("a computed digest parses")
    }

    fn config_json(revision: &str, created: &str, version: &str) -> String {
        serde_json::json!({
            "architecture": "amd64",
            "created": created,
            "config": {"Labels": {
                "org.opencontainers.image.revision": revision,
                "org.opencontainers.image.created": created,
                "org.opencontainers.image.version": version
            }}
        })
        .to_string()
    }

    async fn serve_token(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "t"
            })))
            .mount(server)
            .await;
    }

    /// The stand-in's registry side: HEAD `latest` by a digest, the
    /// manifest at `reference`, and the config blob. The token is
    /// mounted separately where a count on it is the point.
    async fn serve_registry(server: &MockServer, reference: &str, body: &str, config: &str) {
        Mock::given(method("HEAD"))
            .and(path("/v2/marknygaard/niles/manifests/latest"))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("docker-content-digest", sha(reference)),
            )
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v2/marknygaard/niles/manifests/{reference}")))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(
                        body.as_bytes().to_vec(),
                        "application/vnd.oci.image.manifest.v1+json",
                    )
                    .insert_header("docker-content-digest", sha(body)),
            )
            .mount(server)
            .await;
        let config_digest = sha(config);
        Mock::given(method("GET"))
            .and(path(format!("/v2/marknygaard/niles/blobs/{config_digest}")))
            .and(header("authorization", "Bearer t"))
            .respond_with(ResponseTemplate::new(200).set_body_string(config))
            .mount(server)
            .await;
    }

    /// The stand-in's GitHub side: the comparison between the running
    /// commit and `to`, answered `status`.
    async fn serve_compare(server: &MockServer, to: &str, page: serde_json::Value, status: u16) {
        Mock::given(method("GET"))
            .and(path(format!(
                "/repos/MarkNygaard/niles/compare/{RUNNING}...{to}"
            )))
            .respond_with(ResponseTemplate::new(status).set_body_json(page))
            .mount(server)
            .await;
    }

    fn compare_page(ahead_by: u32, behind_by: u32) -> serde_json::Value {
        serde_json::json!({
            "ahead_by": ahead_by,
            "behind_by": behind_by,
            "total_commits": 2,
            "commits": [
                {
                    "sha": RUNNING,
                    "commit": {"message": "feat: first (#510)", "committer": {"date": "2026-11-03T10:00:00Z"}},
                    "parents": [{}],
                },
                {
                    "sha": NEWER,
                    "commit": {"message": "fix: second (#512)", "committer": {"date": "2026-11-04T10:00:00Z"}},
                    "parents": [{}],
                },
            ]
        })
    }

    /// One stand-in serving the registry side, and the check over it.
    async fn stand_in(build: Build, revision: &str, created: &str) -> (MockServer, Arc<Updates>) {
        let server = MockServer::start().await;
        let config = config_json(revision, created, "0.7.0");
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": sha(&config), "size": 3}
        })
        .to_string();
        serve_token(&server).await;
        serve_registry(&server, "latest", &manifest, &config).await;
        let updates = Arc::new(Updates::at(
            build,
            EventBus::default(),
            Registry::at(&server.uri()).expect("the stand-in parses"),
            GitHub::at(&server.uri()).expect("the stand-in parses"),
        ));
        (server, updates)
    }

    #[tokio::test]
    async fn a_dev_build_makes_no_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let updates = Updates::at(
            dev_build(),
            EventBus::default(),
            Registry::at(&server.uri()).expect("the stand-in parses"),
            GitHub::at(&server.uri()).expect("the stand-in parses"),
        );
        let status = updates.check().await;
        assert_eq!(status.state, State::DevBuild);
        assert!(Arc::new(updates).spawn().is_none());
    }

    #[tokio::test]
    async fn the_same_revision_is_up_to_date() {
        // No compare mock is mounted: reaching it would 404 the check.
        let (_server, updates) = stand_in(running_build(), RUNNING, NEWER_CREATED).await;
        let status = updates.check().await;
        assert_eq!(status.state, State::UpToDate);
        assert_eq!(
            status.newest.map(|n| n.commit.as_str().to_string()),
            Some(RUNNING.to_string())
        );
        assert_eq!(status.compare_url, None, "nothing to compare with itself");
        assert!(status.changes.is_none());
    }

    #[tokio::test]
    async fn a_newer_revision_lists_what_changed() {
        let (server, updates) = stand_in(running_build(), NEWER, NEWER_CREATED).await;
        serve_compare(&server, NEWER, compare_page(2, 0), 200).await;
        let status = updates.check().await;
        assert_eq!(status.state, State::UpdateAvailable);
        let changes = status.changes.expect("the comparison is kept");
        assert_eq!(changes.commits.len(), 2);
        assert_eq!(changes.commits[0].subject, "fix: second (#512)");
        assert_eq!(changes.commits[0].pr, Some(512));
        let newest = status.newest.expect("an answer");
        assert_eq!(
            newest.image,
            format!("{}@{}", crate::registry::IMAGE, newest.digest.as_str())
        );
        assert_eq!(
            status.compare_url,
            Some(format!(
                "https://github.com/MarkNygaard/niles/compare/{RUNNING}...{NEWER}"
            ))
        );
    }

    #[tokio::test]
    async fn running_ahead_of_latest_is_up_to_date_and_newer() {
        let (server, updates) = stand_in(running_build(), NEWER, NEWER_CREATED).await;
        serve_compare(&server, NEWER, compare_page(0, 3), 200).await;
        let status = updates.check().await;
        assert_eq!(status.state, State::UpToDate);
        assert!(status.newer_than_published);
    }

    #[tokio::test]
    async fn a_failed_compare_falls_back_to_dates_both_ways() {
        // The newest image is dated after this build: Update available,
        // and the failure is named.
        let (server, updates) = stand_in(running_build(), NEWER, NEWER_CREATED).await;
        serve_compare(&server, NEWER, serde_json::json!({}), 500).await;
        let status = updates.check().await;
        assert_eq!(status.state, State::UpdateAvailable);
        assert!(status.changes.is_none());
        assert!(status.changes_failed.is_some());
        assert!(status.compare_url.is_some());

        // Dated before: Up to date, and this build is the newer one.
        let (server, updates) = stand_in(running_build(), NEWER, "2026-10-01T00:00:00Z").await;
        serve_compare(&server, NEWER, serde_json::json!({}), 500).await;
        let status = updates.check().await;
        assert_eq!(status.state, State::UpToDate);
        assert!(status.newer_than_published);
    }

    #[tokio::test]
    async fn a_registry_failure_with_nothing_kept_says_not_checked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let updates = Updates::at(
            running_build(),
            EventBus::default(),
            Registry::at(&server.uri()).expect("the stand-in parses"),
            GitHub::at(&server.uri()).expect("the stand-in parses"),
        );
        let status = updates.check().await;
        assert_eq!(status.state, State::NotChecked);
        let failed = status.failed.expect("the failure is named");
        assert!(!failed.reason.is_empty());
    }

    #[tokio::test]
    async fn a_failure_after_a_success_keeps_the_answer() {
        let (server, updates) = stand_in(running_build(), NEWER, NEWER_CREATED).await;
        serve_compare(&server, NEWER, compare_page(2, 0), 200).await;
        let first = updates.check().await;
        assert_eq!(first.state, State::UpdateAvailable);
        let checked_at = first.checked_at;
        // The registry goes down; the throttle is off so the second
        // check really tries.
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let updates = Arc::into_inner(updates)
            .expect("one reference")
            .with_intervals(Duration::ZERO, COMPARE_RETRY);
        let updates = Arc::new(updates);
        let second = updates.check().await;
        assert_eq!(
            second.state,
            State::UpdateAvailable,
            "the last answer stands"
        );
        assert!(second.failed.is_some());
        assert_eq!(second.checked_at, checked_at, "kept, not refreshed");
        // And the next success clears the failure. A fresh start, so
        // the 503 token mock from the failure cannot shadow the new
        // one: wiremock matches the first mounted mock.
        server.reset().await;
        let config = config_json(NEWER, NEWER_CREATED, "0.7.0");
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": sha(&config), "size": 3}
        })
        .to_string();
        serve_token(&server).await;
        serve_registry(&server, "latest", &manifest, &config).await;
        serve_compare(&server, NEWER, compare_page(2, 0), 200).await;
        let third = updates.check().await;
        assert!(third.failed.is_none());
        assert!(third.checked_at.is_some());
    }

    #[tokio::test]
    async fn concurrent_checks_are_one_set_of_requests() {
        let server = MockServer::start().await;
        let config = config_json(RUNNING, NEWER_CREATED, "0.7.0");
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": sha(&config), "size": 3}
        })
        .to_string();
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "t"
            })))
            .expect(1)
            .mount(&server)
            .await;
        serve_registry(&server, "latest", &manifest, &config).await;
        let updates = Arc::new(Updates::at(
            running_build(),
            EventBus::default(),
            Registry::at(&server.uri()).expect("the stand-in parses"),
            GitHub::at(&server.uri()).expect("the stand-in parses"),
        ));
        let (a, b) = tokio::join!(updates.check(), updates.check());
        assert_eq!(a.state, State::UpToDate);
        assert_eq!(b.state, State::UpToDate);
    }

    #[tokio::test]
    async fn a_check_within_min_interval_makes_no_requests() {
        let server = MockServer::start().await;
        let config = config_json(RUNNING, NEWER_CREATED, "0.7.0");
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": sha(&config), "size": 3}
        })
        .to_string();
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "t"
            })))
            .expect(1)
            .mount(&server)
            .await;
        serve_registry(&server, "latest", &manifest, &config).await;
        let updates = Updates::at(
            running_build(),
            EventBus::default(),
            Registry::at(&server.uri()).expect("the stand-in parses"),
            GitHub::at(&server.uri()).expect("the stand-in parses"),
        );
        let first = updates.check().await;
        let second = updates.check().await;
        assert_eq!(first.state, State::UpToDate);
        assert_eq!(second.state, State::UpToDate);
    }

    #[tokio::test]
    async fn a_comparison_is_made_once_per_pair() {
        let server = MockServer::start().await;
        let config = config_json(NEWER, NEWER_CREATED, "0.7.0");
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": sha(&config), "size": 3}
        })
        .to_string();
        serve_token(&server).await;
        serve_registry(&server, "latest", &manifest, &config).await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/repos/MarkNygaard/niles/compare/{RUNNING}...{NEWER}"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(compare_page(2, 0)))
            .expect(1)
            .mount(&server)
            .await;
        let updates = Arc::new(
            Updates::at(
                running_build(),
                EventBus::default(),
                Registry::at(&server.uri()).expect("the stand-in parses"),
                GitHub::at(&server.uri()).expect("the stand-in parses"),
            )
            .with_intervals(Duration::ZERO, COMPARE_RETRY),
        );
        let first = updates.check().await;
        assert_eq!(first.state, State::UpdateAvailable);
        // The same digest comes back: the kept comparison for it is
        // not redone.
        let second = updates.check().await;
        assert_eq!(second.state, State::UpdateAvailable);
    }

    #[tokio::test]
    async fn a_failed_comparison_respects_the_retry_window() {
        let server = MockServer::start().await;
        let config = config_json(NEWER, NEWER_CREATED, "0.7.0");
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": sha(&config), "size": 3}
        })
        .to_string();
        serve_token(&server).await;
        serve_registry(&server, "latest", &manifest, &config).await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/repos/MarkNygaard/niles/compare/{RUNNING}...{NEWER}"
            )))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let updates = Arc::new(
            Updates::at(
                running_build(),
                EventBus::default(),
                Registry::at(&server.uri()).expect("the stand-in parses"),
                GitHub::at(&server.uri()).expect("the stand-in parses"),
            )
            .with_intervals(Duration::ZERO, Duration::from_secs(60 * 60)),
        );
        let first = updates.check().await;
        assert!(first.changes_failed.is_some());
        // Within the retry window: not asked again.
        updates.check().await;

        // With the window shut, the next check compares again.
        let server = MockServer::start().await;
        serve_token(&server).await;
        serve_registry(&server, "latest", &manifest, &config).await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/repos/MarkNygaard/niles/compare/{RUNNING}...{NEWER}"
            )))
            .respond_with(ResponseTemplate::new(500))
            .expect(2)
            .mount(&server)
            .await;
        let updates = Arc::new(
            Updates::at(
                running_build(),
                EventBus::default(),
                Registry::at(&server.uri()).expect("the stand-in parses"),
                GitHub::at(&server.uri()).expect("the stand-in parses"),
            )
            .with_intervals(Duration::ZERO, Duration::ZERO),
        );
        updates.check().await;
        updates.check().await;
    }

    #[tokio::test]
    async fn a_check_announces_itself_on_the_bus() {
        let (_server, updates) = stand_in(running_build(), RUNNING, NEWER_CREATED).await;
        let mut events = updates.bus.subscribe();
        let status = updates.check().await;
        assert_eq!(status.state, State::UpToDate);
        let event = events.recv().await.expect("the announcement");
        assert!(matches!(event, Event::UpdateChecked));
    }

    #[test]
    fn the_verdict_rules() {
        let build = running_build();
        let running = build.commit().expect("a commit").clone();
        let newer = Commit::parse(NEWER).expect("40 hex digits");
        let newest = |commit: &Commit, created: &str| Newest {
            digest: digest_of(commit.as_str()),
            image: format!("{}@{}", crate::registry::IMAGE, digest_of(commit.as_str())),
            commit: commit.clone(),
            created: DateTime::parse_from_rfc3339(created)
                .ok()
                .map(|at| at.with_timezone(&Utc)),
            version: Some("0.7.0".to_string()),
        };
        let found = |commit: &Commit, created: &str| Found {
            checked_at: Utc::now(),
            newest: newest(commit, created),
        };
        let compared = |to: &Commit, result: std::result::Result<Changes, String>| Compared {
            to: to.clone(),
            at: Instant::now(),
            result,
        };
        let changes = |ahead: u32, behind: u32| Changes {
            ahead_by: ahead,
            behind_by: behind,
            commits: Vec::new(),
            truncated: false,
        };

        // No commit: a dev build, whatever else is kept.
        assert_eq!(
            verdict(&dev_build(), Some(&found(&running, NEWER_CREATED)), None).state,
            State::DevBuild
        );

        // Nothing found yet.
        assert_eq!(verdict(&build, None, None).state, State::NotChecked);

        // The same published build.
        let same = verdict(&build, Some(&found(&running, NEWER_CREATED)), None);
        assert_eq!(same.state, State::UpToDate);
        assert!(!same.newer_than_published);

        // Compare says ahead.
        let ahead = verdict(
            &build,
            Some(&found(&newer, NEWER_CREATED)),
            Some(&compared(&newer, Ok(changes(2, 0)))),
        );
        assert_eq!(ahead.state, State::UpdateAvailable);
        assert!(!ahead.newer_than_published);

        // Compare says behind.
        let behind = verdict(
            &build,
            Some(&found(&newer, NEWER_CREATED)),
            Some(&compared(&newer, Ok(changes(0, 3)))),
        );
        assert_eq!(behind.state, State::UpToDate);
        assert!(behind.newer_than_published);

        // Compare failed; the newest image is newer by date.
        let failed_then_newer = verdict(
            &build,
            Some(&found(&newer, NEWER_CREATED)),
            Some(&compared(&newer, Err("no".to_string()))),
        );
        assert_eq!(failed_then_newer.state, State::UpdateAvailable);
        assert!(!failed_then_newer.newer_than_published);

        // Compare failed; the newest image is older by date.
        let failed_then_older = verdict(
            &build,
            Some(&found(&newer, "2026-10-01T00:00:00Z")),
            Some(&compared(&newer, Err("no".to_string()))),
        );
        assert_eq!(failed_then_older.state, State::UpToDate);
        assert!(failed_then_older.newer_than_published);

        // No dates at all: a different published build is offered.
        let undated = verdict(&build, Some(&found(&newer, "")), None);
        assert_eq!(undated.state, State::UpdateAvailable);

        // A stale comparison for another commit doesn't decide.
        let stale = verdict(
            &build,
            Some(&found(&newer, NEWER_CREATED)),
            Some(&compared(&running, Ok(changes(0, 0)))),
        );
        assert_eq!(stale.state, State::UpdateAvailable);
    }

    #[test]
    fn a_namespace_is_an_rfc1123_label_or_nothing() {
        assert_eq!(namespace_from("niles\n"), Some("niles".to_string()));
        for bad in ["", "Niles", "a b", "-x", &"a".repeat(64), "x-"] {
            assert_eq!(namespace_from(bad), None, "{bad:?} is not a namespace");
        }
        assert_eq!(
            namespace_from("home-automation"),
            Some("home-automation".to_string())
        );
    }
}
