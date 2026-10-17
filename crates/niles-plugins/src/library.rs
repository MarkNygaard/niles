//! The add-on library: what other people published, for Add-ons →
//! Browse.
//!
//! The library is one file, `index.json`, built by the registry repo
//! [`MarkNygaard/niles-addons`](https://github.com/MarkNygaard/niles-addons)
//! every six hours and on every merge, and served by GitHub Pages at
//! [`INDEX_URL`]. The registry's builder needs no Niles logic: the file
//! holds the repos in its `addons.toml` (Listed), every public repo with
//! the GitHub topic `niles-addon` (Unreviewed), and repos taken off the
//! list (removed, with the reason).
//!
//! Browsing costs one request for that file, kept for six hours, and
//! none of GitHub's 60 API requests an hour: every install by address
//! still works when the library is down.
//!
//! The file is a stranger's, and every part of it is handled the way the
//! install handles the same part: each entry's manifest text goes
//! through the install's own [`Manifest::parse`], so its permissions
//! read as on the approval screen ([`permissions::grants`]) and its
//! `min_niles` is the install's own check, which drives "Needs a newer
//! Niles"; its logo text goes through [`check::sanitise_logo`]. Text is
//! capped, and every link Browse shows is built from the validated repo,
//! never taken from the file.
//!
//! A release's `niles-installed` file exists for this: GitHub's download
//! count of it is the add-on's install count, and [`Installer::install`]
//! downloads it once per repo per house after a fresh install (see
//! [`crate::install`]).
//!
//! ## The format
//!
//! This is the contract with the registry, which the builder writes:
//!
//! ```json
//! {
//!   "version": 1,
//!   "generated_at": "2026-10-17T03:00:00Z",
//!   "addons": [
//!     {
//!       "repo": "johan/niles-withings",
//!       "standing": "listed",
//!       "description": "Withings scales in Niles",
//!       "stars": 12,
//!       "installs": 34,
//!       "downloads": 120,
//!       "release": {
//!         "tag": "v0.3.1",
//!         "published_at": "2026-10-01T12:00:00Z",
//!         "manifest": "id = \"withings\"\nname = \"Withings\"\n…",
//!         "logo": "<svg xmlns=\"http://www.w3.org/2000/svg\" …>…</svg>"
//!       },
//!       "readme": "Weight, body fat and sleep from Withings devices…"
//!     },
//!     { "repo": "jane/niles-old", "standing": "removed", "removed_reason": "Its author archived it." }
//!   ]
//! }
//! ```
//!
//! | Field | What it is |
//! |---|---|
//! | `version` | The format: 1. A change an older Niles would misread bumps it; adding a field doesn't. |
//! | `generated_at` | When the registry built the file, RFC 3339. |
//! | `repo` | `owner/name`. One entry per repo. |
//! | `standing` | `listed` (in `addons.toml`), `unreviewed` (the topic only) or `removed` (taken off the list: Browse hides it, and an installed copy's panel says so). Anything else reads as `unreviewed`. |
//! | `removed_reason` | For `removed`: one sentence, why. |
//! | `description` | The repo's own GitHub description, used when the manifest can't be read. |
//! | `stars` | Stargazers. |
//! | `installs` | Downloads of `niles-installed` summed over every release; `null` when no release has one. |
//! | `downloads` | Downloads of `integration.wasm` summed over every release, previews and updates included. |
//! | `release` | The latest release that is not a draft or a pre-release, or `null`. It has `tag`, `published_at`, `manifest` (its `niles-integration.toml`, as text) and `logo` (the SVG the manifest names, as text, or `null`). |
//! | `readme` | Plain text: the README's opening paragraphs without Markdown, at most 1,000 characters. |
//!
//! Niles's caps: the file is at most [`MAX_INDEX`] (8 MiB) and
//! [`MAX_ADDONS`] (2,000) add-ons. A manifest over [`github::MAX_MANIFEST`]
//! (64 KiB) or a logo over [`check::MAX_LOGO`] (32 KiB) is ignored. Text
//! is cut to 100 characters (name), 300 (description, reason) and 1,000
//! (readme). An entry that doesn't read costs that entry only.

use crate::check;
use crate::github::{self, human_size};
use crate::install::Refusal;
use crate::manifest::{Manifest, NILES_VERSION};
use crate::permissions::{self, CANNOT};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

/// Where the registry's `index.json` is served.
pub const INDEX_URL: &str = "https://marknygaard.github.io/niles-addons/index.json";
/// The release asset whose download count is an add-on's install count.
pub const MARKER: &str = "niles-installed";
/// The marker is a few bytes; a bigger one is never downloaded.
pub const MAX_MARKER: usize = 1024;
/// The index file, at most.
pub const MAX_INDEX: usize = 8 * 1024 * 1024;

/// The format this Niles reads; a higher one says "update Niles".
const FORMAT: u32 = 1;
/// How long a fetched shelf is served before a refresh is tried.
const TTL: Duration = Duration::from_secs(6 * 60 * 60);
/// After a failed refresh, how long the failure is served as-is.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// How long connecting to the index may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long reading the index may take in all.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// The most entries read from one index.
const MAX_ADDONS: usize = 2000;
/// An add-on's name, at most.
const MAX_NAME: usize = 100;
/// A description or a removal reason, at most.
const MAX_TEXT: usize = 300;
/// A README excerpt, at most.
const MAX_README: usize = 1000;

/// The file, as the registry wrote it. Every field but `repo` is lenient,
/// so an odd value costs only itself.
#[derive(Debug, Deserialize)]
struct Index {
    version: u32,
    #[serde(default)]
    generated_at: Option<String>,
    #[serde(default)]
    addons: Vec<serde_json::Value>,
}

/// One entry, before it is judged. Unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct IndexEntry {
    repo: String,
    #[serde(default)]
    standing: Option<String>,
    #[serde(default)]
    removed_reason: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stars: u64,
    #[serde(default)]
    installs: Option<u64>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    release: Option<IndexRelease>,
    #[serde(default)]
    readme: Option<String>,
}

/// The latest release an entry carries, as text.
#[derive(Debug, Deserialize)]
struct IndexRelease {
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    manifest: Option<String>,
    #[serde(default)]
    logo: Option<String>,
}

/// Where an add-on stands with the library's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Standing {
    /// In the registry's `addons.toml`: passed the checks when listed
    /// and read by someone. Not an audit of its code.
    Listed,
    /// Found by the `niles-addon` topic only. Nobody has checked it.
    Unreviewed,
    /// Taken off the list: hidden from Browse, and an installed copy's
    /// panel says so.
    Removed,
}

/// The library as Browse shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Shelf {
    /// When the registry built the index this copy came from.
    pub generated_at: Option<DateTime<Utc>>,
    /// What no add-on may do, always the same sentence.
    pub cannot: &'static str,
    pub addons: Vec<ShelfEntry>,
}

/// One add-on in the library, already judged.
#[derive(Debug, Clone, Serialize)]
pub struct ShelfEntry {
    /// `https://github.com/owner/name`, built from the checked repo.
    pub repo: String,
    pub owner: String,
    pub standing: Standing,
    /// For `Removed`: why it was taken off the list.
    pub removed_reason: Option<String>,
    pub name: String,
    pub description: String,
    /// The manifest's version.
    pub version: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    /// Downloads of `niles-installed`; `None` when no release has one.
    pub installs: Option<u64>,
    /// Downloads of the component, previews and updates included.
    pub downloads: u64,
    pub stars: u64,
    /// The sanitised SVG itself.
    pub logo: Option<String>,
    pub readme: Option<String>,
    /// What it may do, in the approval screen's words; `None` when this
    /// Niles cannot read its manifest. Never guessed.
    pub permissions: Option<Vec<String>>,
    pub min_niles: Option<String>,
    /// Above this Niles's version: it cannot be installed here.
    pub needs_newer_niles: bool,
    /// Why Browse won't install it, as a sentence.
    pub blocked: Option<String>,
}

/// Trim, take at most `max` chars (char-safe), trim the end again.
fn capped(text: &str, max: usize) -> String {
    text.trim()
        .chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The first line of `text`, before capping.
fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// What judging an entry's release found: what Browse shows, and
/// whether it can be installed here.
struct Judged {
    name: Option<String>,
    description: Option<String>,
    version: Option<String>,
    permissions: Option<Vec<String>>,
    min_niles: Option<String>,
    needs_newer_niles: bool,
    blocked: Option<String>,
}

/// Judge an entry's release: read its manifest with the install's own
/// parser, so permissions and `min_niles` read exactly as on the
/// approval screen. A manifest this Niles cannot parse is still read
/// loosely, so one that names a newer Niles says so.
fn judge(release: Option<&IndexRelease>) -> Judged {
    let mut judged = Judged {
        name: None,
        description: None,
        version: None,
        permissions: None,
        min_niles: None,
        needs_newer_niles: false,
        blocked: None,
    };
    let Some(release) = release else {
        judged.blocked = Some("It has no release yet.".into());
        return judged;
    };
    let tag_version = || {
        release
            .tag
            .as_deref()
            .map(|t| t.strip_prefix('v').unwrap_or(t).into())
    };
    let Some(manifest) = release
        .manifest
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        judged.version = tag_version();
        judged.blocked = Some("Its latest release has no niles-integration.toml.".into());
        return judged;
    };
    if manifest.len() > github::MAX_MANIFEST {
        judged.version = tag_version();
        judged.blocked = Some("Its manifest is larger than 64 KiB.".into());
        return judged;
    }
    let this = semver::Version::parse(NILES_VERSION).expect("our own version");
    match Manifest::parse(manifest) {
        Ok(m) => {
            judged.permissions = Some(
                permissions::grants(&m, None)
                    .into_iter()
                    .map(|g| g.text)
                    .collect(),
            );
            if let Ok(min) = semver::Version::parse(&m.min_niles)
                && min > this
            {
                judged.needs_newer_niles = true;
                judged.blocked = Some(format!(
                    "It needs Niles {min} or newer, and this is Niles {this}."
                ));
            } else if !m.problems().is_empty() {
                judged.blocked = Some(format!(
                    "This Niles would not install it: {}.",
                    m.problems().join("; ")
                ));
            }
            judged.name = Some(m.name);
            judged.description = Some(m.description);
            judged.version = Some(m.version);
            judged.min_niles = Some(m.min_niles);
        }
        Err(e) => {
            // The manifest is deny_unknown_fields, so one written for a
            // newer Niles fails to parse: read it loosely, to still say
            // which Niles it needs.
            let loose = toml::from_str::<toml::Table>(manifest).ok();
            let string = |key: &str| {
                loose
                    .as_ref()
                    .and_then(|t| t.get(key))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            judged.name = string("name");
            judged.description = string("description");
            judged.version = string("version").or_else(tag_version);
            judged.min_niles = string("min_niles");
            match judged
                .min_niles
                .as_deref()
                .and_then(|s| semver::Version::parse(s).ok())
            {
                Some(min) if min > this => {
                    judged.needs_newer_niles = true;
                    judged.blocked = Some(format!(
                        "It needs Niles {min} or newer, and this is Niles {this}."
                    ));
                }
                _ => judged.blocked = Some(format!("This Niles cannot read its manifest: {e}.")),
            }
        }
    }
    judged
}

fn rank(standing: Standing) -> u8 {
    match standing {
        Standing::Removed => 2,
        Standing::Listed => 1,
        Standing::Unreviewed => 0,
    }
}

/// Read one raw entry into what Browse shows. `None` when its repo
/// doesn't read, or named a tag: the library lists repos, not releases.
fn entry(raw: IndexEntry) -> Option<ShelfEntry> {
    let (repo, tag) = github::parse_repo(&raw.repo).ok()?;
    if tag.is_some() {
        return None;
    }
    let standing = match raw.standing.as_deref() {
        Some("listed") => Standing::Listed,
        Some("removed") => Standing::Removed,
        _ => Standing::Unreviewed,
    };
    let judged = judge(raw.release.as_ref());
    let description = judged
        .description
        .as_deref()
        .or(raw.description.as_deref())
        .map(first_line)
        .unwrap_or("");
    let logo = raw
        .release
        .as_ref()
        .and_then(|r| r.logo.as_ref())
        .and_then(|svg| {
            (svg.len() <= check::MAX_LOGO).then(|| check::sanitise_logo(svg.as_bytes()).ok())?
        });
    let readme = raw
        .readme
        .as_deref()
        .map(|r| capped(r, MAX_README))
        .filter(|r| !r.is_empty());
    let published_at = raw
        .release
        .as_ref()
        .and_then(|r| r.published_at.as_deref())
        .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
        .map(|d| d.with_timezone(&Utc));
    Some(ShelfEntry {
        repo: repo.url(),
        owner: repo.owner,
        standing,
        removed_reason: (standing == Standing::Removed)
            .then(|| {
                raw.removed_reason
                    .as_deref()
                    .map(|r| capped(first_line(r), MAX_TEXT))
            })
            .flatten(),
        name: capped(judged.name.as_deref().unwrap_or(&repo.name), MAX_NAME),
        description: capped(description, MAX_TEXT),
        version: judged.version,
        published_at,
        installs: raw.installs,
        downloads: raw.downloads,
        stars: raw.stars,
        logo,
        readme,
        permissions: judged.permissions,
        min_niles: judged.min_niles,
        needs_newer_niles: judged.needs_newer_niles,
        blocked: judged.blocked,
    })
}

/// Read index bytes into a shelf. A wrong format refuses; a bad entry
/// costs that entry only.
fn read(bytes: &[u8]) -> Result<Shelf, Refusal> {
    let index: Index = serde_json::from_slice(bytes)
        .map_err(|e| Refusal::Upstream(format!("The add-on library did not read: {e}")))?;
    if index.version != FORMAT {
        if index.version > FORMAT {
            return Err(Refusal::Upstream(format!(
                "The add-on library is in a newer format (version {}) than this Niles reads. \
                 Update Niles to browse it; installing by address still works.",
                index.version
            )));
        }
        return Err(Refusal::Upstream(format!(
            "The add-on library did not read: its version {} is unknown.",
            index.version
        )));
    }
    let generated_at = index
        .generated_at
        .as_deref()
        .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
        .map(|d| d.with_timezone(&Utc));
    let mut addons: Vec<ShelfEntry> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut skipped = 0usize;
    for raw in index.addons.into_iter().take(MAX_ADDONS) {
        let Some(read) = (|| {
            let raw = serde_json::from_value::<IndexEntry>(raw).ok()?;
            entry(raw)
        })() else {
            skipped += 1;
            continue;
        };
        // One entry per repo, by lowercase owner/name. A removal is the
        // safety signal: it wins over anything else, in the first
        // position it appeared at.
        let key = read.repo.to_lowercase();
        match at.get(&key) {
            Some(&i) => {
                if rank(read.standing) > rank(addons[i].standing) {
                    addons[i] = read;
                }
            }
            None => {
                at.insert(key, addons.len());
                addons.push(read);
            }
        }
    }
    if skipped > 0 {
        tracing::debug!("[plugins] the add-on library: {skipped} entries did not read");
    }
    Ok(Shelf {
        generated_at,
        cannot: CANNOT,
        addons,
    })
}

/// The add-on library: fetches the registry's `index.json`, keeps it,
/// and reads it as a stranger's.
pub struct Library {
    client: reqwest::Client,
    url: Url,
    ttl: Duration,
    retry_after: Duration,
    held: tokio::sync::Mutex<Held>,
}

#[derive(Default)]
struct Held {
    shelf: Option<(Instant, Arc<Shelf>)>,
    failed: Option<(Instant, Refusal)>,
}

impl Default for Library {
    fn default() -> Self {
        Self::at(INDEX_URL).expect("the index's own address parses")
    }
}

impl Library {
    /// The library at `url`: the registry's own address, or a stand-in
    /// in tests.
    pub fn at(url: &str) -> Result<Self, String> {
        let url = Url::parse(url).map_err(|e| format!("{url}: {e}"))?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("niles/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            url,
            ttl: TTL,
            retry_after: RETRY_AFTER,
            held: Default::default(),
        })
    }

    /// The shelf as Browse shows it. One fetch serves every caller
    /// while the copy held is fresh; a failed refresh serves the stale
    /// copy, and is not retried for [`RETRY_AFTER`].
    pub async fn shelf(&self) -> Result<Arc<Shelf>, Refusal> {
        // Held across the fetch: that is the single flight.
        let mut held = self.held.lock().await;
        if let Some((at, shelf)) = &held.shelf
            && at.elapsed() < self.ttl
        {
            return Ok(shelf.clone());
        }
        if let Some((at, failure)) = &held.failed
            && at.elapsed() < self.retry_after
        {
            return held
                .shelf
                .as_ref()
                .map_or_else(|| Err(failure.clone()), |(_, shelf)| Ok(shelf.clone()));
        }
        match self.fetch().await {
            Ok(shelf) => {
                let shelf = Arc::new(shelf);
                held.shelf = Some((Instant::now(), shelf.clone()));
                held.failed = None;
                Ok(shelf)
            }
            Err(why) => {
                tracing::warn!("[plugins] the add-on library: {why}");
                held.failed = Some((Instant::now(), why));
                held.shelf.as_ref().map_or_else(
                    || Err(held.failed.as_ref().expect("just failed").1.clone()),
                    |(_, shelf)| Ok(shelf.clone()),
                )
            }
        }
    }

    async fn fetch(&self) -> Result<Shelf, Refusal> {
        let response = self
            .client
            .get(self.url.clone())
            .header("accept", "application/json")
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                Refusal::Upstream(format!("The add-on library could not be reached: {e}"))
            })?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(Refusal::Upstream(format!(
                "The add-on library was not found at {}.",
                self.url
            )));
        }
        if !status.is_success() {
            return Err(Refusal::Upstream(format!(
                "The add-on library answered {status}; try again in a while."
            )));
        }
        let bytes = github::read_capped(response, MAX_INDEX, "The add-on library").await?;
        // Parsing every manifest and sanitising every logo is CPU work
        // that must not hold an async worker (house rule 5).
        let shelf = tokio::task::spawn_blocking(move || read(&bytes))
            .await
            .map_err(|e| Refusal::Unavailable(format!("reading the add-on library: {e}")))??;
        tracing::debug!(
            "[plugins] the add-on library: {} add-ons",
            shelf.addons.len()
        );
        Ok(shelf)
    }
}

/// What the add-on library needs of a release before it is listed,
/// beyond what an install checks.
pub fn listing_problems(release: &github::Release) -> Vec<String> {
    match release.asset(MARKER) {
        None => vec![format!(
            "the release has no {MARKER} asset, which the add-on library counts installs by"
        )],
        Some(asset) if asset.size > MAX_MARKER as u64 => vec![format!(
            "{MARKER} is {}; it should be a few bytes, at most {}",
            human_size(asset.size),
            human_size(MAX_MARKER as u64)
        )],
        Some(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HELLO: &str = include_str!("../tests/fixtures/hello/niles-integration.toml");
    const LOGO: &str = r#"<?xml version="1.0"?>
<!-- the hello mark -->
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="currentColor"/></svg>"#;

    fn shelf_of(value: serde_json::Value) -> Shelf {
        read(&value.to_string().into_bytes()).unwrap_or_else(|e| panic!("{e}"))
    }

    fn listed(manifest: &str) -> serde_json::Value {
        json!({
            "version": 1,
            "addons": [{
                "repo": "jane/hello",
                "standing": "listed",
                "description": "From the repo",
                "stars": 3,
                "installs": 2,
                "downloads": 9,
                "release": {"tag": "v0.1.0", "manifest": manifest},
            }],
        })
    }

    #[test]
    fn a_listed_add_on_reads_in_the_approval_screens_words() {
        let value = json!({
            "version": 1,
            "addons": [{
                "repo": "jane/hello",
                "standing": "listed",
                "release": {
                    "tag": "v0.1.0",
                    "manifest": HELLO,
                    "logo": LOGO,
                },
                "readme": "Says hello.",
            }],
        });
        let shelf = shelf_of(value);
        let entry = &shelf.addons[0];
        assert_eq!(entry.name, "Hello");
        assert_eq!(entry.standing, Standing::Listed);
        assert_eq!(entry.blocked, None);
        assert!(!entry.needs_newer_niles);
        let manifest = Manifest::parse(HELLO).unwrap();
        let texts = permissions::grants(&manifest, None)
            .into_iter()
            .map(|g| g.text)
            .collect::<Vec<_>>();
        assert_eq!(entry.permissions, Some(texts));
        assert_eq!(
            entry.permissions.as_ref().unwrap()[0],
            "Connect to example.com"
        );
        assert_eq!(entry.repo, "https://github.com/jane/hello");
        assert_eq!(entry.owner, "jane");
        assert_eq!(
            entry.logo,
            Some(check::sanitise_logo(LOGO.as_bytes()).unwrap())
        );
    }

    #[test]
    fn one_for_a_newer_niles_cannot_be_installed_and_says_why() {
        let manifest = HELLO.replacen("min_niles = \"0.2.0\"", "min_niles = \"99.0.0\"", 1);
        let shelf = shelf_of(listed(&manifest));
        let entry = &shelf.addons[0];
        assert!(entry.needs_newer_niles);
        let blocked = entry.blocked.as_deref().unwrap();
        assert!(blocked.contains("99.0.0"), "{blocked}");
        assert!(blocked.contains(NILES_VERSION), "{blocked}");
        assert_eq!(entry.min_niles.as_deref(), Some("99.0.0"));
    }

    #[test]
    fn a_manifest_this_niles_cannot_read_still_says_which_niles_it_needs() {
        // A field from a newer Niles fails deny_unknown_fields, the way
        // the manifest would in the wild.
        let manifest = HELLO
            .replacen("min_niles = \"0.2.0\"", "min_niles = \"99.0.0\"", 1)
            .replacen(
                "interface = \"0.1\"",
                "interface = \"0.1\"\nfuture_field = true",
                1,
            );
        let shelf = shelf_of(listed(&manifest));
        let entry = &shelf.addons[0];
        assert!(entry.needs_newer_niles, "{:?}", entry.blocked);
        assert_eq!(entry.permissions, None);
        assert_eq!(entry.name, "Hello", "the loose read still finds the name");
    }

    #[test]
    fn an_unreadable_manifest_shows_no_permissions_and_says_why() {
        let shelf = shelf_of(listed("not toml ["));
        let entry = &shelf.addons[0];
        let blocked = entry.blocked.as_deref().unwrap();
        assert!(
            blocked.starts_with("This Niles cannot read its manifest"),
            "{blocked}"
        );
        assert_eq!(entry.permissions, None);
        assert!(!entry.needs_newer_niles);
        assert_eq!(entry.description, "From the repo");
    }

    #[test]
    fn one_bad_entry_costs_that_entry_not_the_library() {
        let shelf = shelf_of(json!({
            "version": 1,
            "addons": [
                {"repo": "not a repo"},
                {"repo": 7},
                {"repo": "jane/x/releases/tag/v1"},
                {"repo": "jane/ok"},
            ],
        }));
        assert_eq!(shelf.addons.len(), 1);
        assert_eq!(shelf.addons[0].repo, "https://github.com/jane/ok");
    }

    #[test]
    fn text_is_capped_and_links_come_from_the_repo() {
        let readme = "r".repeat(5000);
        let description = format!("{}\\nsecond line", "d".repeat(400));
        let manifest = format!(
            "id = \"x\"\nname = \"X\"\nversion = \"0.1.0\"\ndescription = \"{description}\"\n\
             authors = [\"a\"]\nmin_niles = \"0.2.0\"\ninterface = \"0.1\"\n"
        );
        let shelf = shelf_of(json!({
            "version": 1,
            "addons": [{
                "repo": "https://github.com/Jane/Hello",
                "release": {"tag": "v0.1.0", "manifest": manifest},
                "readme": readme,
            }],
        }));
        let entry = &shelf.addons[0];
        assert_eq!(entry.readme.as_ref().unwrap().chars().count(), MAX_README);
        assert_eq!(entry.description, "d".repeat(MAX_TEXT));
        assert_eq!(entry.repo, "https://github.com/Jane/Hello");
    }

    #[test]
    fn an_unknown_standing_reads_as_unreviewed_and_a_removal_keeps_its_reason() {
        let shelf = shelf_of(json!({
            "version": 1,
            "addons": [
                {"repo": "jane/a", "standing": "featured"},
                {"repo": "jane/b", "standing": "removed", "removed_reason": "Gone."},
                {"repo": "jane/c", "standing": "listed", "removed_reason": "Ignored."},
            ],
        }));
        assert_eq!(shelf.addons[0].standing, Standing::Unreviewed);
        assert_eq!(shelf.addons[1].standing, Standing::Removed);
        assert_eq!(shelf.addons[1].removed_reason.as_deref(), Some("Gone."));
        assert_eq!(shelf.addons[2].removed_reason, None);
    }

    #[test]
    fn the_same_repo_twice_is_kept_once_and_a_removal_wins() {
        let shelf = shelf_of(json!({
            "version": 1,
            "addons": [
                {"repo": "jane/hello", "standing": "unreviewed"},
                {"repo": "Jane/Hello", "standing": "removed"},
            ],
        }));
        assert_eq!(shelf.addons.len(), 1);
        assert_eq!(shelf.addons[0].standing, Standing::Removed);
    }

    #[test]
    fn a_newer_format_is_refused_with_a_way_forward() {
        let answer = read(&json!({"version": 2, "addons": []}).to_string().into_bytes());
        match answer.unwrap_err() {
            Refusal::Upstream(why) => assert!(why.contains("Update Niles"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_logo_goes_through_the_install_allowlist() {
        let evil = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><script>alert(1)</script><circle cx="12" cy="12" r="9"/></svg>"#;
        let shelf = shelf_of(json!({
            "version": 1,
            "addons": [{
                "repo": "jane/hello",
                "release": {"tag": "v0.1.0", "manifest": HELLO, "logo": evil},
            }],
        }));
        let out = shelf.addons[0].logo.clone().unwrap_or_default();
        assert!(!out.contains("script"), "{out}");
    }

    #[test]
    fn listing_needs_a_small_marker() {
        let release = |assets: Vec<github::Asset>| github::Release {
            tag_name: "v0.1.0".into(),
            html_url: String::new(),
            body: None,
            draft: false,
            prerelease: false,
            assets,
        };
        let marker = |size: u64| github::Asset {
            name: MARKER.into(),
            size,
            browser_download_url: String::new(),
            digest: None,
        };
        assert_eq!(listing_problems(&release(Vec::new())).len(), 1);
        let big = listing_problems(&release(vec![marker(8 * 1024)]));
        assert_eq!(big.len(), 1);
        assert!(big[0].contains("8 KiB"), "{big:?}");
        assert!(listing_problems(&release(vec![marker(16)])).is_empty());
    }

    mod http {
        use super::*;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn index() -> serde_json::Value {
            json!({
                "version": 1,
                "addons": [{"repo": "jane/hello", "standing": "listed"}],
            })
        }

        async fn server_with(index: serde_json::Value) -> (MockServer, Library) {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/index.json"))
                .respond_with(ResponseTemplate::new(200).set_body_json(index))
                .mount(&server)
                .await;
            let library = Library::at(&format!("{}/index.json", server.uri())).unwrap();
            (server, library)
        }

        #[tokio::test]
        async fn the_library_is_fetched_once_while_it_is_fresh() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/index.json"))
                .respond_with(ResponseTemplate::new(200).set_body_json(index()))
                .expect(1)
                .mount(&server)
                .await;
            let library = Library::at(&format!("{}/index.json", server.uri())).unwrap();
            let first = library.shelf().await.unwrap();
            let second = library.shelf().await.unwrap();
            assert_eq!(first.addons.len(), 1);
            assert!(
                Arc::ptr_eq(&first, &second),
                "the copy kept is served, not fetched again"
            );
        }

        #[tokio::test]
        async fn a_failed_refresh_serves_what_was_there() {
            let (server, mut library) = server_with(index()).await;
            let first = library.shelf().await.unwrap();
            library.ttl = Duration::ZERO;
            server.reset().await;
            Mock::given(method("GET"))
                .and(path("/index.json"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;
            let served = library.shelf().await.unwrap();
            assert!(
                Arc::ptr_eq(&first, &served),
                "the stale copy is served when a refresh fails"
            );
            let fresh = Library::at(&format!("{}/index.json", server.uri())).unwrap();
            assert!(
                matches!(fresh.shelf().await, Err(Refusal::Upstream(_))),
                "with nothing kept, the failure is the answer"
            );
        }

        #[tokio::test]
        async fn a_failure_is_not_retried_at_once() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/index.json"))
                .respond_with(ResponseTemplate::new(500))
                .expect(1)
                .mount(&server)
                .await;
            let library = Library::at(&format!("{}/index.json", server.uri())).unwrap();
            assert!(matches!(library.shelf().await, Err(Refusal::Upstream(_))));
            assert!(
                matches!(library.shelf().await, Err(Refusal::Upstream(_))),
                "the failure is served, not fetched again"
            );
        }

        #[tokio::test]
        async fn a_missing_library_says_where_it_looked() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/index.json"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;
            let library = Library::at(&format!("{}/index.json", server.uri())).unwrap();
            match library.shelf().await.unwrap_err() {
                Refusal::Upstream(why) => {
                    assert!(why.contains("/index.json"), "{why}");
                    assert!(why.contains(&server.uri()), "{why}");
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
