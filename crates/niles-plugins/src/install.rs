//! Installing plugins from GitHub, keeping them up to date, and taking
//! them away again.
//!
//! An install is two steps, so what the household approves is exactly
//! what runs. A **preview** downloads the release, checks everything
//! `niles plugin check` checks and everything that would clash with what
//! is already in the house, and keeps the files in a staging area under
//! a random token for [`Options::staging_ttl`]. It answers with what the
//! approval screen shows: the plugin, its permissions in plain words,
//! the settings it will ask for. The **install** takes that token, checks
//! the files against the sha256 pinned at the preview, and hands them to
//! [`Runtime::install_package`]. Nothing is downloaded twice, so a release
//! asset replaced on GitHub in between changes nothing.
//!
//! Updates take the same two steps from the plugin's panel, with the
//! permission difference shown. A background check looks for newer
//! releases every [`Options::check_every`] and records what it finds.
//! For [`Options::rollback_window`] after an update the new version is
//! watched, and if it traps, times out, or its component does not load,
//! the version before it is put back.

use crate::check::{self, MAX_LOGO};
use crate::github::{self, GitHub, MAX_MANIFEST, Release, Repo};
use crate::manifest::{Manifest, SettingKind};
use crate::permissions::{self, CANNOT};
use crate::runtime::{MAX_COMPONENT, Runtime, Status};
use crate::store::{Package, Record, Source, Update};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_tools::ToolDescriptor;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Why an install, update or removal did not happen, as a sentence for
/// the household, sorted by whose move it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// What was sent is not a repo, a token or a value Niles can use.
    BadRequest(String),
    /// No such repo, release or plugin.
    NotFound(String),
    /// It would clash with what is installed, or the plugin is not in a
    /// state where this makes sense.
    Conflict(String),
    /// The release is not a plugin this Niles would run: the manifest,
    /// the component, the logo or an asset is wrong.
    Invalid(String),
    /// The preview's token has expired or was used.
    Gone(String),
    /// GitHub did not answer, or refused.
    Upstream(String),
    /// This Niles cannot install plugins: it has no database.
    Unavailable(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::BadRequest(s)
            | Refusal::NotFound(s)
            | Refusal::Conflict(s)
            | Refusal::Invalid(s)
            | Refusal::Gone(s)
            | Refusal::Upstream(s)
            | Refusal::Unavailable(s) => f.write_str(s),
        }
    }
}

impl From<crate::Error> for Refusal {
    fn from(error: crate::Error) -> Self {
        match error {
            crate::Error::UnknownPlugin(_) => Refusal::NotFound(error.to_string()),
            crate::Error::NoPrevious(_) | crate::Error::AlreadyRunning(_) => {
                Refusal::Conflict(error.to_string())
            }
            crate::Error::Manifest(_) | crate::Error::Component(_) => {
                Refusal::Invalid(error.to_string())
            }
            _ => Refusal::Unavailable(error.to_string()),
        }
    }
}

type Outcome<T> = std::result::Result<T, Refusal>;

/// The intervals and limits of the install flow.
#[derive(Debug, Clone)]
pub struct Options {
    /// How long a preview's files are kept for its install.
    pub staging_ttl: Duration,
    /// The most bytes kept in staging at once; past it the oldest
    /// preview is dropped.
    pub staging_bytes: usize,
    /// How long after an update a failure rolls it back.
    pub rollback_window: Duration,
    /// How often an updated plugin is looked at during that window.
    pub watch_every: Duration,
    /// How often every plugin's repo is asked for a newer release.
    pub check_every: Duration,
    /// How long after startup the first check runs, so a Niles that
    /// restarts in a loop does not spend GitHub's hourly limit.
    pub first_check_after: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            staging_ttl: Duration::from_secs(15 * 60),
            staging_bytes: 192 * 1024 * 1024,
            rollback_window: Duration::from_secs(10 * 60),
            watch_every: Duration::from_secs(2),
            check_every: Duration::from_secs(6 * 60 * 60),
            first_check_after: Duration::from_secs(2 * 60),
        }
    }
}

/// What a removed plugin leaves outside the plugin tables: its settings
/// in the config store, its secrets in the secret store, its menu
/// entries. The binary knows where those are.
#[async_trait]
pub trait Leftovers: Send + Sync {
    async fn forget(&self, id: &str) -> Result<(), String>;
}

/// A catalogue group, as the Integrations page heads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub id: String,
    pub label: String,
}

/// The group a manifest names, or Custom.
pub fn group_of(manifest: &Manifest) -> Group {
    manifest
        .group
        .as_deref()
        .and_then(|wanted| {
            niles_config::catalogue::GROUPS.iter().find(|g| {
                serde_json::to_value(g.id)
                    .ok()
                    .is_some_and(|id| id.as_str() == Some(wanted))
            })
        })
        .map(|g| Group {
            id: manifest.group.clone().unwrap_or_default(),
            label: g.label.to_string(),
        })
        .unwrap_or_else(|| Group {
            id: "custom".into(),
            label: "Custom".into(),
        })
}

/// What the approval screen shows.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    /// Give it back to install exactly this.
    pub token: String,
    /// Seconds until the token expires.
    pub expires_in: u64,
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub authors: Vec<String>,
    pub homepage: Option<String>,
    /// `https://github.com/<owner>/<repo>`.
    pub repo: String,
    pub tag: String,
    pub release_url: String,
    pub notes: String,
    pub group: Group,
    /// The sanitised SVG.
    pub logo: Option<String>,
    /// What it may do, one line each.
    pub permissions: Vec<String>,
    /// What it may not, always.
    pub cannot: &'static str,
    pub tools: Vec<ToolPreview>,
    /// What its setup panel will ask for.
    pub settings: Vec<SettingPreview>,
    pub warnings: Vec<String>,
    /// The component's sha256, pinned by this preview.
    pub sha256: String,
    pub manifest_sha256: String,
    /// For an update: the version installed now.
    pub current_version: Option<String>,
    /// For an update: permissions it adds and drops.
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolPreview {
    /// The bare name, as the permission line lists it.
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingPreview {
    pub key: String,
    pub label: String,
    /// `string`, `number`, `boolean`, `secret`, `choice` or `room`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub required: bool,
    pub help: Option<String>,
}

/// One installed plugin, as the Integrations page lists it.
#[derive(Debug, Clone)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub homepage: Option<String>,
    pub group: Group,
    /// `starting`, `running`, `not_set_up`, `failing`, `stopped` or
    /// `disabled`.
    pub status: String,
    pub status_line: String,
    pub error: Option<String>,
    pub dropped_events: u64,
    pub enabled: bool,
    pub source: Option<Source>,
    pub installed_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub has_logo: bool,
    /// The version a rollback goes back to.
    pub previous_version: Option<String>,
    pub update: Option<Update>,
}

/// Installs, updates and removes plugins. Cheap to clone.
#[derive(Clone)]
pub struct Installer {
    inner: Arc<Inner>,
}

struct Inner {
    runtime: Runtime,
    github: GitHub,
    options: Options,
    database: bool,
    /// Niles's own tools' names, which no plugin's may take.
    reserved_tools: Vec<String>,
    leftovers: Option<Arc<dyn Leftovers>>,
    staged: Mutex<Vec<Staged>>,
    /// What happened to a plugin that its status does not say: an update
    /// that was rolled back.
    notices: Mutex<HashMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Purpose {
    Install,
    Update { id: String, from: String },
}

struct Staged {
    token: String,
    at: Instant,
    purpose: Purpose,
    package: Package,
    sha256: String,
    manifest_sha256: String,
}

/// A release downloaded and checked.
struct Fetched {
    release: Release,
    manifest: Manifest,
    text: String,
    component: Vec<u8>,
    logo: Option<String>,
    tools: Vec<ToolDescriptor>,
    warnings: Vec<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Installer {
    pub fn new(runtime: Runtime, github: GitHub, options: Options) -> Self {
        Self {
            inner: Arc::new(Inner {
                runtime,
                github,
                options,
                database: true,
                reserved_tools: Vec::new(),
                leftovers: None,
                staged: Mutex::default(),
                notices: Mutex::default(),
            }),
        }
    }

    fn edit(mut self, f: impl FnOnce(&mut Inner)) -> Self {
        f(Arc::get_mut(&mut self.inner).expect("set up before it is shared"));
        self
    }

    /// Whether installs are kept anywhere a restart keeps them. Without
    /// a database nothing can be installed, and a preview says so.
    pub fn with_database(self, database: bool) -> Self {
        self.edit(|inner| inner.database = database)
    }

    /// Niles's own tools, whose names a plugin's tools may not take.
    pub fn with_reserved_tools(self, names: Vec<String>) -> Self {
        self.edit(|inner| inner.reserved_tools = names)
    }

    pub fn with_leftovers(self, leftovers: Arc<dyn Leftovers>) -> Self {
        self.edit(|inner| inner.leftovers = Some(leftovers))
    }

    pub fn runtime(&self) -> &Runtime {
        &self.inner.runtime
    }

    async fn records(&self) -> Outcome<Vec<Record>> {
        self.inner
            .runtime
            .store()
            .records()
            .await
            .map_err(Refusal::Unavailable)
    }

    async fn record(&self, id: &str) -> Outcome<Record> {
        self.records()
            .await?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| Refusal::NotFound(format!("There is no plugin called {id}.")))
    }

    /// Download the release `repo` names (the latest, or `tag`), check
    /// it, and keep it for [`install`](Self::install).
    pub async fn preview(&self, repo: &str, tag: Option<&str>) -> Outcome<Preview> {
        if !self.inner.database {
            return Err(Refusal::Unavailable(
                "Plugins need Niles's database, and this Niles has none: an installed plugin \
                 would be gone after a restart. Set up [database] first."
                    .into(),
            ));
        }
        let (repo, tag_in_url) = github::parse_repo(repo).map_err(Refusal::BadRequest)?;
        let tag = match tag.map(str::trim).filter(|t| !t.is_empty()) {
            Some(tag) => {
                github::check_tag(tag).map_err(Refusal::BadRequest)?;
                Some(tag.to_string())
            }
            None => tag_in_url,
        };
        let fetched = self.fetch(&repo, tag.as_deref()).await?;
        let records = self.records().await?;
        if let Some(installed) = records.iter().find(|r| r.id == fetched.manifest.id) {
            return Err(Refusal::Conflict(format!(
                "{} is already installed (version {}). Update it from its panel instead.",
                fetched.manifest.name, installed.version
            )));
        }
        self.clashes(&fetched, false)?;
        Ok(self.stage(&repo, fetched, Purpose::Install))
    }

    /// Install what the preview with `token` downloaded. `sha256`, when
    /// given, is the component's as the preview showed it, and must
    /// still be.
    pub async fn install(&self, token: &str, sha256: Option<&str>) -> Outcome<Listed> {
        let staged = self.take(token, |p| *p == Purpose::Install)?;
        verify(&staged, sha256)?;
        let manifest = Manifest::parse(&staged.package.manifest)
            .map_err(|e| Refusal::Invalid(e.to_string()))?;
        if self.records().await?.iter().any(|r| r.id == manifest.id) {
            return Err(Refusal::Conflict(format!(
                "{} was installed since the preview.",
                manifest.name
            )));
        }
        self.inner.runtime.install_package(staged.package).await?;
        tracing::info!(plugin = %manifest.id, "[plugins] installed {} {}", manifest.name, manifest.version);
        self.listed(&manifest.id).await
    }

    /// Ask `id`'s repo for a newer release and record what it finds:
    /// the newest by semver, ignoring pre-releases unless the installed
    /// version is one.
    pub async fn check_update(&self, id: &str) -> Outcome<Option<Update>> {
        let record = self.record(id).await?;
        let source = record.source.clone().ok_or_else(|| {
            Refusal::Conflict(format!(
                "{id} was not installed from GitHub, so there is nowhere to look for updates."
            ))
        })?;
        let (repo, _) = github::parse_repo(&source.repo).map_err(Refusal::Invalid)?;
        let installed = semver::Version::parse(&record.version)
            .map_err(|e| Refusal::Invalid(format!("the installed version: {e}")))?;
        let pre = !installed.pre.is_empty();
        let newest = self
            .inner
            .github
            .releases(&repo)
            .await?
            .into_iter()
            .filter(|r| !r.draft && (pre || !r.prerelease))
            .filter_map(|r| r.version().map(|v| (v, r)))
            .filter(|(v, _)| (pre || v.pre.is_empty()) && *v > installed)
            .max_by(|(a, _), (b, _)| a.cmp(b));
        let store = self.inner.runtime.store();
        let Some((version, release)) = newest else {
            store
                .set_update(id, None)
                .await
                .map_err(Refusal::Unavailable)?;
            return Ok(None);
        };
        let asset = release.asset(MANIFEST_ASSET).ok_or_else(|| {
            Refusal::Invalid(format!(
                "Release {} has no {MANIFEST_ASSET}.",
                release.tag_name
            ))
        })?;
        let text = self.inner.github.download(asset, MAX_MANIFEST).await?;
        let text = String::from_utf8(text)
            .map_err(|_| Refusal::Invalid(format!("{MANIFEST_ASSET} is not UTF-8 text.")))?;
        let next = Manifest::parse(&text).map_err(|e| Refusal::Invalid(e.to_string()))?;
        let now = Manifest::parse(&record.manifest).map_err(|e| Refusal::Invalid(e.to_string()))?;
        let diff = permissions::diff(
            &permissions::grants(&now, None),
            &permissions::grants(&next, None),
        );
        let update = Update {
            version: version.to_string(),
            tag: release.tag_name.clone(),
            release_url: release.html_url.clone(),
            notes: release.body.clone().unwrap_or_default(),
            added: diff.added,
            removed: diff.removed,
        };
        store
            .set_update(id, Some(&update))
            .await
            .map_err(Refusal::Unavailable)?;
        Ok(Some(update))
    }

    /// Check every plugin installed from GitHub. Stops at GitHub's rate
    /// limit; the rest wait for the next round.
    pub async fn check_all(&self) {
        let Ok(records) = self.records().await else {
            return;
        };
        for record in records.iter().filter(|r| r.source.is_some()) {
            match self.check_update(&record.id).await {
                Ok(Some(update)) => tracing::info!(
                    plugin = %record.id,
                    "[plugins] {} is available (installed: {})",
                    update.version,
                    record.version
                ),
                Ok(None) => {}
                Err(Refusal::Upstream(why)) => {
                    tracing::warn!("[plugins] update check: {why}");
                    return;
                }
                Err(why) => tracing::warn!(plugin = %record.id, "[plugins] update check: {why}"),
            }
        }
    }

    /// Check for updates shortly after startup, then every
    /// [`Options::check_every`].
    pub fn spawn_update_checks(&self) -> tokio::task::JoinHandle<()> {
        let installer = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(installer.inner.options.first_check_after).await;
            loop {
                installer.check_all().await;
                tokio::time::sleep(installer.inner.options.check_every).await;
            }
        })
    }

    /// Download the newer release the last check found (checking again
    /// if it found none), and keep it for [`update`](Self::update).
    pub async fn update_preview(&self, id: &str) -> Outcome<Preview> {
        let record = self.record(id).await?;
        let source = record.source.clone().ok_or_else(|| {
            Refusal::Conflict(format!(
                "{id} was not installed from GitHub, so it is not updated from there."
            ))
        })?;
        let (repo, _) = github::parse_repo(&source.repo).map_err(Refusal::Invalid)?;
        let update = match record.update.clone() {
            Some(update) => update,
            None => self.check_update(id).await?.ok_or_else(|| {
                Refusal::Conflict(format!("{id} {} is the newest release.", record.version))
            })?,
        };
        let fetched = self.fetch(&repo, Some(&update.tag)).await?;
        if fetched.manifest.id != id {
            return Err(Refusal::Invalid(format!(
                "Release {} is a plugin called {}, not {id}.",
                update.tag, fetched.manifest.id
            )));
        }
        let newer = semver::Version::parse(&fetched.manifest.version)
            .ok()
            .zip(semver::Version::parse(&record.version).ok())
            .is_some_and(|(new, old)| new > old);
        if !newer {
            return Err(Refusal::Conflict(format!(
                "Release {} is not newer than the installed {}.",
                update.tag, record.version
            )));
        }
        self.clashes(&fetched, true)?;
        let before =
            Manifest::parse(&record.manifest).map_err(|e| Refusal::Invalid(e.to_string()))?;
        let names = bare_names(&fetched.manifest.id, &fetched.tools);
        let diff = permissions::diff(
            &permissions::grants(&before, None),
            &permissions::grants(&fetched.manifest, Some(&names)),
        );
        let mut preview = self.stage(
            &repo,
            fetched,
            Purpose::Update {
                id: id.to_string(),
                from: record.version.clone(),
            },
        );
        preview.current_version = Some(record.version);
        preview.added = diff.added;
        preview.removed = diff.removed;
        Ok(preview)
    }

    /// Install what the update preview with `token` downloaded, keep the
    /// version it replaces, and watch the new one.
    pub async fn update(&self, id: &str, token: &str, sha256: Option<&str>) -> Outcome<Listed> {
        let staged = self.take(
            token,
            |p| matches!(p, Purpose::Update { id: of, .. } if of == id),
        )?;
        verify(&staged, sha256)?;
        let Purpose::Update { from, .. } = &staged.purpose else {
            unreachable!("taken as an update");
        };
        let record = self.record(id).await?;
        if &record.version != from {
            return Err(Refusal::Conflict(format!(
                "{id} changed since the preview (it is {} now); preview the update again.",
                record.version
            )));
        }
        let version = Manifest::parse(&staged.package.manifest)
            .map_err(|e| Refusal::Invalid(e.to_string()))?
            .version;
        self.inner.runtime.install_package(staged.package).await?;
        lock(&self.inner.notices).remove(id);
        tracing::info!(plugin = %id, "[plugins] updated from {} to {version}", record.version);
        self.watch(id.to_string(), version);
        self.listed(id).await
    }

    /// Roll an update back if the new version fails within the window.
    fn watch(&self, id: String, version: String) {
        let installer = self.clone();
        tokio::spawn(async move {
            let options = &installer.inner.options;
            let until = Instant::now() + options.rollback_window;
            while Instant::now() < until {
                tokio::time::sleep(options.watch_every).await;
                let Some(info) = installer.inner.runtime.info(&id) else {
                    return;
                };
                if info.version != version {
                    return;
                }
                if info.failures == 0 {
                    continue;
                }
                let why = info.error.unwrap_or_else(|| "it failed".into());
                match installer.inner.runtime.roll_back(&id).await {
                    Ok(back) => {
                        let notice = format!(
                            "Version {version} failed soon after the update ({why}), so {back} \
                             was put back."
                        );
                        tracing::warn!(plugin = %id, "[plugins] {notice}");
                        lock(&installer.inner.notices).insert(id, notice);
                    }
                    Err(e) => {
                        tracing::error!(plugin = %id, "[plugins] {version} failed ({why}) and could not be rolled back: {e}")
                    }
                }
                return;
            }
        });
    }

    /// Put the version before the last update back.
    pub async fn roll_back(&self, id: &str) -> Outcome<Listed> {
        let version = self.inner.runtime.roll_back(id).await?;
        lock(&self.inner.notices).remove(id);
        tracing::info!(plugin = %id, "[plugins] rolled back to {version}");
        self.listed(id).await
    }

    pub async fn enable(&self, id: &str) -> Outcome<Listed> {
        self.inner.runtime.enable(id).await?;
        self.listed(id).await
    }

    pub async fn disable(&self, id: &str) -> Outcome<Listed> {
        self.inner.runtime.disable(id).await?;
        self.listed(id).await
    }

    /// Stop and forget a plugin: its devices, tools, storage and stored
    /// component, and through [`Leftovers`] its settings and secrets.
    pub async fn remove(&self, id: &str) -> Outcome<()> {
        self.inner.runtime.remove(id).await?;
        lock(&self.inner.notices).remove(id);
        lock(&self.inner.staged)
            .retain(|s| !matches!(&s.purpose, Purpose::Update { id: of, .. } if of == id));
        if let Some(leftovers) = &self.inner.leftovers
            && let Err(e) = leftovers.forget(id).await
        {
            // The plugin is gone; what is left is somebody's to tidy, not
            // a reason to say the removal failed.
            tracing::warn!(plugin = %id, "[plugins] removed, but its settings or secrets: {e}");
        }
        tracing::info!(plugin = %id, "[plugins] removed");
        Ok(())
    }

    /// Stop the plugin's instance and start a fresh one, as a change to
    /// its settings or secrets does.
    pub fn restart(&self, id: &str) {
        let _ = self.inner.runtime.restart(id);
    }

    pub async fn logo(&self, id: &str) -> Outcome<Option<String>> {
        self.inner
            .runtime
            .store()
            .logo(id)
            .await
            .map_err(Refusal::Unavailable)
    }

    /// Every installed plugin's declared secrets, as the secret store
    /// keys them (`plugins.<id>.<key>`), with a label for each.
    pub async fn secret_settings(&self) -> Vec<(String, String)> {
        let Ok(records) = self.records().await else {
            return Vec::new();
        };
        records
            .iter()
            .filter_map(|r| Manifest::parse(&r.manifest).ok())
            .flat_map(|m| {
                m.settings
                    .iter()
                    .filter(|s| s.kind == SettingKind::Secret)
                    .map(|s| {
                        (
                            format!("plugins.{}.{}", m.id, s.key),
                            format!("{}: {}", m.name, s.label),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Every installed plugin, oldest first, with how it is doing.
    pub async fn list(&self) -> Vec<Listed> {
        let records = match self.records().await {
            Ok(records) => records,
            Err(e) => {
                tracing::warn!("[plugins] {e}");
                Vec::new()
            }
        };
        let running = self.inner.runtime.plugins();
        let notices = lock(&self.inner.notices).clone();
        let mut out: Vec<Listed> = records
            .into_iter()
            .filter_map(|record| {
                let manifest = Manifest::parse(&record.manifest).ok()?;
                let info = running.iter().find(|p| p.id == record.id);
                let (status, status_line, error, dropped) = match (record.enabled, info) {
                    (false, _) => ("disabled".to_string(), "Off".to_string(), None, 0),
                    (true, Some(info)) => (
                        info.status.kind().to_string(),
                        info.status.line(),
                        info.error.clone(),
                        info.dropped_events,
                    ),
                    (true, None) => {
                        let status = Status::Failing("it could not be started".into());
                        (status.kind().to_string(), status.line(), None, 0)
                    }
                };
                Some(Listed {
                    id: record.id.clone(),
                    name: manifest.name.clone(),
                    version: record.version,
                    description: manifest.description.clone(),
                    authors: manifest.authors.clone(),
                    homepage: manifest.homepage.clone(),
                    group: group_of(&manifest),
                    status,
                    status_line,
                    error: notices.get(&record.id).cloned().or(error),
                    dropped_events: dropped,
                    enabled: record.enabled,
                    source: record.source,
                    installed_at: Some(record.installed_at),
                    updated_at: Some(record.updated_at),
                    has_logo: record.has_logo,
                    previous_version: record.previous_version,
                    update: record.update,
                })
            })
            .collect();
        // Running without a record: the store could not be read.
        for info in running {
            if out.iter().any(|l| l.id == info.id) {
                continue;
            }
            out.push(Listed {
                id: info.id.clone(),
                name: info.name.clone(),
                version: info.version.clone(),
                description: String::new(),
                authors: Vec::new(),
                homepage: None,
                group: Group {
                    id: "custom".into(),
                    label: "Custom".into(),
                },
                status: info.status.kind().to_string(),
                status_line: info.status.line(),
                error: info.error.clone(),
                dropped_events: info.dropped_events,
                enabled: true,
                source: None,
                installed_at: None,
                updated_at: None,
                has_logo: false,
                previous_version: None,
                update: None,
            });
        }
        out
    }

    async fn listed(&self, id: &str) -> Outcome<Listed> {
        self.list()
            .await
            .into_iter()
            .find(|l| l.id == id)
            .ok_or_else(|| Refusal::NotFound(format!("There is no plugin called {id}.")))
    }

    /// Download and check a release, without keeping anything.
    async fn fetch(&self, repo: &Repo, tag: Option<&str>) -> Outcome<Fetched> {
        let github = &self.inner.github;
        let release = github.release(repo, tag).await?;
        let missing = |name: &str| {
            Refusal::Invalid(format!(
                "Release {} of {} has no {name} among its assets, so it is not a Niles plugin \
                 release.",
                release.tag_name,
                repo.slug()
            ))
        };
        let manifest_asset = release
            .asset(MANIFEST_ASSET)
            .ok_or_else(|| missing(MANIFEST_ASSET))?;
        let component_asset = release
            .asset(COMPONENT_ASSET)
            .ok_or_else(|| missing(COMPONENT_ASSET))?;
        let text = github.download(manifest_asset, MAX_MANIFEST).await?;
        let text = String::from_utf8(text)
            .map_err(|_| Refusal::Invalid(format!("{MANIFEST_ASSET} is not UTF-8 text.")))?;
        let manifest = Manifest::parse(&text).map_err(|e| Refusal::Invalid(e.to_string()))?;
        let tagged = release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name);
        if tagged != manifest.version {
            return Err(Refusal::Invalid(format!(
                "The release is tagged {}, and its manifest says version {}. They must be the \
                 same, with or without a v.",
                release.tag_name, manifest.version
            )));
        }
        // Said before the component is downloaded: a plugin for a newer
        // Niles is refused without fetching 64 MiB to find out.
        let problems = manifest.problems();
        if !problems.is_empty() {
            return Err(Refusal::Invalid(format!(
                "The manifest is not one this Niles accepts: {}.",
                problems.join("; ")
            )));
        }
        let logo = match &manifest.logo {
            None => None,
            Some(path) => {
                let name = path.rsplit('/').next().unwrap_or(path);
                let asset = release.asset(name).ok_or_else(|| missing(name))?;
                Some(github.download(asset, MAX_LOGO).await?)
            }
        };
        let component = github.download(component_asset, MAX_COMPONENT).await?;
        let report = check::check(&text, Some(component.clone()), logo.clone()).await;
        if !report.ok() {
            return Err(Refusal::Invalid(format!(
                "It would not run: {}.",
                report.problems.join("; ")
            )));
        }
        let logo = logo
            .map(|svg| check::sanitise_logo(&svg))
            .transpose()
            .map_err(|why| Refusal::Invalid(format!("logo: {why}")))?;
        let mut warnings = Vec::new();
        if manifest.logo.is_none() {
            warnings.push("It has no logo of its own; its group's icon stands in.".into());
        }
        if component_asset.digest.is_none() {
            warnings.push(format!(
                "GitHub reports no checksum for {COMPONENT_ASSET}, so the download could only \
                 be pinned, not compared."
            ));
        }
        let missing = manifest.missing_required(&toml::Table::new(), |_| false);
        if !missing.is_empty() {
            warnings.push(format!(
                "It starts once these are filled in: {}.",
                missing.join(", ")
            ));
        }
        if manifest
            .permissions
            .local_network
            .iter()
            .any(|e| e.contains('/'))
        {
            warnings.push(
                "It may reach a whole range of addresses on your home network, not one device."
                    .into(),
            );
        }
        Ok(Fetched {
            release,
            manifest,
            text,
            component,
            logo,
            tools: report.tools,
            warnings,
        })
    }

    /// What in the house the fetched plugin would collide with.
    fn clashes(&self, fetched: &Fetched, updating: bool) -> Outcome<()> {
        let id = &fetched.manifest.id;
        let mut clashes = Vec::new();
        let others: HashMap<String, String> = self
            .inner
            .runtime
            .dynamic_tools()
            .names()
            .into_iter()
            .filter(|(owner, _)| owner != id)
            .map(|(owner, name)| (name, owner))
            .collect();
        for tool in &fetched.tools {
            if self.inner.reserved_tools.contains(&tool.name) {
                clashes.push(format!(
                    "its tool {} has the name of one of Niles's own tools",
                    tool.name
                ));
            } else if let Some(owner) = others.get(&tool.name) {
                clashes.push(format!(
                    "its tool {} has the name of one of {owner}'s tools",
                    tool.name
                ));
            }
        }
        let source = fetched.manifest.source();
        if !updating
            && self
                .inner
                .runtime
                .registry()
                .list_all()
                .iter()
                .any(|d| d.id.source() == source)
        {
            clashes.push(format!(
                "devices under the source {source} are already in the house"
            ));
        }
        if clashes.is_empty() {
            Ok(())
        } else {
            Err(Refusal::Conflict(format!(
                "{} cannot be installed beside what is here: {}.",
                fetched.manifest.name,
                clashes.join("; ")
            )))
        }
    }

    /// Keep `fetched` under a fresh token and describe it.
    fn stage(&self, repo: &Repo, fetched: Fetched, purpose: Purpose) -> Preview {
        let Fetched {
            release,
            manifest,
            text,
            component,
            logo,
            tools,
            warnings,
        } = fetched;
        let token = format!("{:032x}", rand::random::<u128>());
        let sha256 = niles_db::hex_digest(&component);
        let manifest_sha256 = niles_db::hex_digest(text.as_bytes());
        let names = bare_names(&manifest.id, &tools);
        let preview = Preview {
            token: token.clone(),
            expires_in: self.inner.options.staging_ttl.as_secs(),
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            version: manifest.version.clone(),
            authors: manifest.authors.clone(),
            homepage: manifest.homepage.clone(),
            repo: repo.url(),
            tag: release.tag_name.clone(),
            release_url: release.html_url.clone(),
            notes: release.body.clone().unwrap_or_default(),
            group: group_of(&manifest),
            logo: logo.clone(),
            permissions: permissions::grants(&manifest, Some(&names))
                .into_iter()
                .map(|g| g.text)
                .collect(),
            cannot: CANNOT,
            tools: tools
                .iter()
                .zip(&names)
                .map(|(t, name)| ToolPreview {
                    name: name.clone(),
                    description: t.description.clone(),
                })
                .collect(),
            settings: manifest
                .settings
                .iter()
                .map(|s| SettingPreview {
                    key: s.key.clone(),
                    label: s.label.clone(),
                    kind: kind_name(s.kind),
                    required: s.required,
                    help: s.help.clone(),
                })
                .collect(),
            warnings,
            sha256: sha256.clone(),
            manifest_sha256: manifest_sha256.clone(),
            current_version: None,
            added: Vec::new(),
            removed: Vec::new(),
        };
        let staged = Staged {
            token,
            at: Instant::now(),
            purpose,
            package: Package {
                manifest: text,
                component,
                logo,
                source: Some(Source {
                    repo: repo.url(),
                    tag: release.tag_name,
                    release_url: release.html_url,
                }),
            },
            sha256,
            manifest_sha256,
        };
        let mut all = lock(&self.inner.staged);
        let ttl = self.inner.options.staging_ttl;
        all.retain(|s| s.at.elapsed() < ttl);
        let size = |s: &Staged| s.package.component.len() + s.package.manifest.len();
        let mut held: usize = all.iter().map(size).sum();
        while !all.is_empty() && held + size(&staged) > self.inner.options.staging_bytes {
            held -= size(&all.remove(0));
        }
        all.push(staged);
        preview
    }

    /// The staged preview with `token`, taken out of staging: a token
    /// installs once.
    fn take(&self, token: &str, fits: impl Fn(&Purpose) -> bool) -> Outcome<Staged> {
        let mut all = lock(&self.inner.staged);
        let ttl = self.inner.options.staging_ttl;
        all.retain(|s| s.at.elapsed() < ttl);
        let Some(at) = all.iter().position(|s| s.token == token) else {
            return Err(Refusal::Gone(
                "That preview has expired or was already used. Preview it again.".into(),
            ));
        };
        if !fits(&all[at].purpose) {
            return Err(Refusal::BadRequest(
                "That preview is for something else.".into(),
            ));
        }
        Ok(all.remove(at))
    }
}

const MANIFEST_ASSET: &str = "niles-integration.toml";
const COMPONENT_ASSET: &str = "integration.wasm";

/// The files are what the preview pinned, and, when the caller says
/// which component it approved, that one.
fn verify(staged: &Staged, approved: Option<&str>) -> Outcome<()> {
    let component = niles_db::hex_digest(&staged.package.component);
    let manifest = niles_db::hex_digest(staged.package.manifest.as_bytes());
    if component != staged.sha256 || manifest != staged.manifest_sha256 {
        return Err(Refusal::Conflict(
            "The downloaded files changed after the preview. Preview it again.".into(),
        ));
    }
    if let Some(approved) = approved
        && !approved.trim().eq_ignore_ascii_case(&component)
    {
        return Err(Refusal::Conflict(format!(
            "The component's sha256 is {component}, not the {} that was approved. Preview it \
             again.",
            approved.trim()
        )));
    }
    Ok(())
}

/// Tool names without the `<id>_` the model sees them with.
fn bare_names(id: &str, tools: &[ToolDescriptor]) -> Vec<String> {
    let prefix = format!("{id}_");
    tools
        .iter()
        .map(|t| t.name.strip_prefix(&prefix).unwrap_or(&t.name).to_string())
        .collect()
}

fn kind_name(kind: SettingKind) -> &'static str {
    match kind {
        SettingKind::String => "string",
        SettingKind::Number => "number",
        SettingKind::Boolean => "boolean",
        SettingKind::Secret => "secret",
        SettingKind::Choice => "choice",
        SettingKind::Room => "room",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(group: Option<&str>) -> Manifest {
        let group = group
            .map(|g| format!("group = \"{g}\"\n"))
            .unwrap_or_default();
        Manifest::parse(&format!(
            "{group}id = \"x\"\nname = \"X\"\nversion = \"0.1.0\"\ndescription = \"x\"\n\
             authors = [\"a\"]\nmin_niles = \"0.2.0\"\ninterface = \"0.1\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn a_known_group_is_kept_and_anything_else_is_custom() {
        assert_eq!(
            group_of(&manifest(Some("presence"))),
            Group {
                id: "presence".into(),
                label: "Presence & climate".into()
            }
        );
        assert_eq!(group_of(&manifest(Some("pools"))).id, "custom");
        assert_eq!(group_of(&manifest(None)).label, "Custom");
    }

    fn staged(component: &[u8]) -> Staged {
        Staged {
            token: "t".into(),
            at: Instant::now(),
            purpose: Purpose::Install,
            package: Package {
                manifest: "id = \"x\"".into(),
                component: component.to_vec(),
                logo: None,
                source: None,
            },
            sha256: niles_db::hex_digest(component),
            manifest_sha256: niles_db::hex_digest(b"id = \"x\""),
        }
    }

    #[test]
    fn what_is_installed_is_what_was_pinned() {
        let good = staged(b"\0asm");
        assert_eq!(verify(&good, None), Ok(()));
        assert_eq!(verify(&good, Some(&good.sha256.to_uppercase())), Ok(()));
        assert!(matches!(
            verify(&good, Some(&"0".repeat(64))),
            Err(Refusal::Conflict(_))
        ));
        let mut changed = staged(b"\0asm");
        changed.package.component.push(0);
        assert!(matches!(verify(&changed, None), Err(Refusal::Conflict(_))));
    }
}
