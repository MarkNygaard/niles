//! Where installed plugins, their private storage, and the devices Niles
//! holds for them are kept.
//!
//! Postgres when Niles has a database ([`niles_db::PostgresPlugins`]);
//! in memory otherwise, and for `niles plugin dev`, which can keep the
//! storage in a file beside the plugin so a restart keeps it.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_core::{Device, DeviceClass, DeviceId, DeviceState, LightCapabilities};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;

/// All of a plugin's keys and values together.
pub const STORAGE_QUOTA: usize = 1024 * 1024;
/// The longest key.
pub const MAX_KEY: usize = 256;

/// A plugin as installed: its approved manifest and its component.
#[derive(Debug, Clone)]
pub struct Installed {
    pub manifest: String,
    pub component: Vec<u8>,
}

/// What an approved install or update keeps.
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: String,
    pub component: Vec<u8>,
    /// The sanitised SVG.
    pub logo: Option<String>,
    /// Where it came from; none for a file installed by hand.
    pub source: Option<Source>,
}

/// The GitHub release a plugin was installed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// `https://github.com/<owner>/<repo>`.
    pub repo: String,
    /// Its name in a repo of several add-ons, whose releases are tagged
    /// `<addon>-vX.Y.Z`; `None` for a repo of one.
    #[serde(default)]
    pub addon: Option<String>,
    pub tag: String,
    /// The release's page.
    pub release_url: String,
}

/// A newer release the update check found, and what it would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    pub version: String,
    pub tag: String,
    pub release_url: String,
    /// The release's notes, as its author wrote them.
    #[serde(default)]
    pub notes: String,
    /// Permissions it asks for that the installed version does not, in
    /// plain words.
    #[serde(default)]
    pub added: Vec<String>,
    /// Permissions it no longer asks for.
    #[serde(default)]
    pub removed: Vec<String>,
}

/// An installed plugin as listed: everything but the component.
#[derive(Debug, Clone)]
pub struct Record {
    pub id: String,
    pub version: String,
    pub manifest: String,
    pub source: Option<Source>,
    pub sha256: String,
    pub has_logo: bool,
    pub enabled: bool,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The version a rollback would go back to.
    pub previous_version: Option<String>,
    pub update: Option<Update>,
    /// What happened to it that its status does not say: an update that
    /// failed and was rolled back. Kept until the next update or rollback
    /// by hand, so the panel still says why after a restart.
    pub notice: Option<String>,
}

/// A device Niles holds for a plugin, so it outlives the instance.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownDevice {
    /// `room/name`, as the plugin names it.
    pub id: String,
    pub class: DeviceClass,
    pub light: LightCapabilities,
    pub controllable: bool,
    /// Whether it is one of its room's lights; see `devices.set-room-light`.
    pub room_light: bool,
    /// Whether it has reported an action, so the rule editor offers it
    /// as a trigger after a restart too.
    pub reports_actions: bool,
}

impl KnownDevice {
    /// The device as the registry holds it, with `state`.
    pub(crate) fn device(&self, id: DeviceId, state: DeviceState) -> Device {
        let mut device = Device::new(id, state, self.class).with_capabilities(self.light);
        device.room_light = self.room_light;
        device.reports_actions = self.reports_actions;
        device
    }
}

/// Whether a `set` was kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Yes,
    /// It would have taken the plugin past [`STORAGE_QUOTA`].
    OverQuota,
}

#[async_trait]
pub trait PluginStore: Send + Sync {
    /// The enabled plugins, to start with Niles.
    async fn installed(&self) -> Result<Vec<Installed>, String>;
    /// Every plugin, enabled or not.
    async fn records(&self) -> Result<Vec<Record>, String>;
    /// One plugin's manifest and component, enabled or not.
    async fn package(&self, id: &str) -> Result<Option<Installed>, String>;
    /// Keep a plugin, replacing one with the same id and keeping what it
    /// replaced for one rollback. Enabled, with no update pending.
    /// Internal: what an approved install writes, after the caller has
    /// checked it.
    async fn install(&self, id: &str, version: &str, package: &Package) -> Result<(), String>;
    /// Put the previous version back, forgetting the current one. `None`
    /// when there is no previous version.
    async fn roll_back(&self, id: &str) -> Result<Option<Installed>, String>;
    /// False when there is no such plugin.
    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, String>;
    async fn set_update(&self, id: &str, update: Option<&Update>) -> Result<(), String>;
    /// Record what happened to the plugin, or forget it. An install,
    /// an update and a rollback forget it on their own.
    async fn set_notice(&self, id: &str, notice: Option<&str>) -> Result<(), String>;
    /// Note that this house counted an install from `repo` (`owner/name`,
    /// lowercase): true the first time, false ever after, even once the
    /// plugin is removed, so installing it again is not counted again.
    async fn count_install(&self, repo: &str) -> Result<bool, String>;
    async fn logo(&self, id: &str) -> Result<Option<String>, String>;
    /// Forget the plugin, its storage and its devices.
    async fn remove(&self, id: &str) -> Result<(), String>;

    async fn get(&self, plugin: &str, key: &str) -> Result<Option<String>, String>;
    async fn set(&self, plugin: &str, key: &str, value: &str) -> Result<Stored, String>;
    async fn delete(&self, plugin: &str, key: &str) -> Result<(), String>;
    /// Keys starting with `prefix`, sorted.
    async fn keys(&self, plugin: &str, prefix: &str) -> Result<Vec<String>, String>;

    async fn devices(&self, plugin: &str) -> Result<Vec<KnownDevice>, String>;
    async fn put_device(&self, plugin: &str, device: &KnownDevice) -> Result<(), String>;
    async fn remove_device(&self, plugin: &str, id: &str) -> Result<(), String>;
}

/// Everything in memory, for a Niles without a database and for
/// `niles plugin dev`.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Memory>,
    /// Where the storage is written after each change, if anywhere.
    file: Option<PathBuf>,
}

#[derive(Default)]
struct Memory {
    installed: BTreeMap<String, Kept>,
    storage: BTreeMap<String, BTreeMap<String, String>>,
    devices: BTreeMap<String, BTreeMap<String, KnownDevice>>,
    counted: BTreeSet<String>,
}

struct Kept {
    version: String,
    package: Package,
    previous: Option<(String, Package)>,
    enabled: bool,
    installed_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    update: Option<Update>,
    notice: Option<String>,
}

fn installed(package: &Package) -> Installed {
    Installed {
        manifest: package.manifest.clone(),
        component: package.component.clone(),
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Storage kept in `file` as JSON, read now and written after every
    /// change, so a dev run that restarts keeps what the plugin stored.
    /// An unreadable file starts empty.
    pub fn in_file(file: PathBuf) -> Self {
        let storage = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(Memory {
                storage,
                ..Default::default()
            }),
            file: Some(file),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, memory: &Memory) -> Result<(), String> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(&memory.storage).map_err(|e| e.to_string())?;
        std::fs::write(file, text).map_err(|e| format!("{}: {e}", file.display()))
    }
}

#[async_trait]
impl PluginStore for MemoryStore {
    async fn installed(&self) -> Result<Vec<Installed>, String> {
        Ok(self
            .lock()
            .installed
            .values()
            .filter(|k| k.enabled)
            .map(|k| installed(&k.package))
            .collect())
    }

    async fn records(&self) -> Result<Vec<Record>, String> {
        Ok(self
            .lock()
            .installed
            .iter()
            .map(|(id, k)| Record {
                id: id.clone(),
                version: k.version.clone(),
                manifest: k.package.manifest.clone(),
                source: k.package.source.clone(),
                sha256: niles_db::hex_digest(&k.package.component),
                has_logo: k.package.logo.is_some(),
                enabled: k.enabled,
                installed_at: k.installed_at,
                updated_at: k.updated_at,
                previous_version: k.previous.as_ref().map(|(v, _)| v.clone()),
                update: k.update.clone(),
                notice: k.notice.clone(),
            })
            .collect())
    }

    async fn package(&self, id: &str) -> Result<Option<Installed>, String> {
        Ok(self.lock().installed.get(id).map(|k| installed(&k.package)))
    }

    async fn install(&self, id: &str, version: &str, package: &Package) -> Result<(), String> {
        let mut memory = self.lock();
        let now = Utc::now();
        let previous = memory.installed.remove(id);
        let installed_at = previous.as_ref().map_or(now, |k| k.installed_at);
        memory.installed.insert(
            id.to_string(),
            Kept {
                version: version.to_string(),
                package: package.clone(),
                previous: previous.map(|k| (k.version, k.package)),
                enabled: true,
                installed_at,
                updated_at: now,
                update: None,
                notice: None,
            },
        );
        Ok(())
    }

    async fn roll_back(&self, id: &str) -> Result<Option<Installed>, String> {
        let mut memory = self.lock();
        let Some(kept) = memory.installed.get_mut(id) else {
            return Ok(None);
        };
        let Some((version, package)) = kept.previous.take() else {
            return Ok(None);
        };
        kept.version = version;
        kept.package = package;
        kept.update = None;
        kept.notice = None;
        kept.updated_at = Utc::now();
        Ok(Some(installed(&kept.package)))
    }

    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, String> {
        Ok(match self.lock().installed.get_mut(id) {
            Some(kept) => {
                kept.enabled = enabled;
                true
            }
            None => false,
        })
    }

    async fn set_update(&self, id: &str, update: Option<&Update>) -> Result<(), String> {
        if let Some(kept) = self.lock().installed.get_mut(id) {
            kept.update = update.cloned();
        }
        Ok(())
    }

    async fn set_notice(&self, id: &str, notice: Option<&str>) -> Result<(), String> {
        if let Some(kept) = self.lock().installed.get_mut(id) {
            kept.notice = notice.map(str::to_string);
        }
        Ok(())
    }

    async fn count_install(&self, repo: &str) -> Result<bool, String> {
        Ok(self.lock().counted.insert(repo.to_string()))
    }

    async fn logo(&self, id: &str) -> Result<Option<String>, String> {
        Ok(self
            .lock()
            .installed
            .get(id)
            .and_then(|k| k.package.logo.clone()))
    }

    async fn remove(&self, id: &str) -> Result<(), String> {
        let mut memory = self.lock();
        memory.installed.remove(id);
        memory.storage.remove(id);
        memory.devices.remove(id);
        self.save(&memory)
    }

    async fn get(&self, plugin: &str, key: &str) -> Result<Option<String>, String> {
        Ok(self
            .lock()
            .storage
            .get(plugin)
            .and_then(|s| s.get(key))
            .cloned())
    }

    async fn set(&self, plugin: &str, key: &str, value: &str) -> Result<Stored, String> {
        let mut memory = self.lock();
        let others: usize = memory
            .storage
            .get(plugin)
            .map(|s| {
                s.iter()
                    .filter(|(k, _)| *k != key)
                    .map(|(k, v)| k.len() + v.len())
                    .sum()
            })
            .unwrap_or(0);
        if others + key.len() + value.len() > STORAGE_QUOTA {
            return Ok(Stored::OverQuota);
        }
        memory
            .storage
            .entry(plugin.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
        self.save(&memory)?;
        Ok(Stored::Yes)
    }

    async fn delete(&self, plugin: &str, key: &str) -> Result<(), String> {
        let mut memory = self.lock();
        if let Some(s) = memory.storage.get_mut(plugin) {
            s.remove(key);
        }
        self.save(&memory)
    }

    async fn keys(&self, plugin: &str, prefix: &str) -> Result<Vec<String>, String> {
        Ok(self
            .lock()
            .storage
            .get(plugin)
            .map(|s| {
                s.keys()
                    .filter(|k| k.starts_with(prefix))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn devices(&self, plugin: &str) -> Result<Vec<KnownDevice>, String> {
        Ok(self
            .lock()
            .devices
            .get(plugin)
            .map(|d| d.values().cloned().collect())
            .unwrap_or_default())
    }

    async fn put_device(&self, plugin: &str, device: &KnownDevice) -> Result<(), String> {
        self.lock()
            .devices
            .entry(plugin.to_string())
            .or_default()
            .insert(device.id.clone(), device.clone());
        Ok(())
    }

    async fn remove_device(&self, plugin: &str, id: &str) -> Result<(), String> {
        if let Some(d) = self.lock().devices.get_mut(plugin) {
            d.remove(id);
        }
        Ok(())
    }
}

fn from_row(row: niles_db::PluginRow) -> Installed {
    Installed {
        manifest: row.manifest,
        component: row.component,
    }
}

#[async_trait]
impl PluginStore for niles_db::PostgresPlugins {
    async fn installed(&self) -> Result<Vec<Installed>, String> {
        Ok(self.enabled().await?.into_iter().map(from_row).collect())
    }

    async fn records(&self) -> Result<Vec<Record>, String> {
        Ok(self
            .all()
            .await?
            .into_iter()
            .map(|row| Record {
                source: match (row.repo, row.release_tag) {
                    (Some(repo), Some(tag)) => Some(Source {
                        repo,
                        addon: row.addon,
                        tag,
                        release_url: row.release_url.unwrap_or_default(),
                    }),
                    _ => None,
                },
                // A record that does not read is no update: the next
                // check writes it again.
                update: row
                    .update_available
                    .and_then(|json| serde_json::from_str(&json).ok()),
                id: row.id,
                version: row.version,
                manifest: row.manifest,
                sha256: row.sha256,
                has_logo: row.has_logo,
                enabled: row.enabled,
                installed_at: row.installed_at,
                updated_at: row.updated_at,
                previous_version: row.previous_version,
                notice: row.notice,
            })
            .collect())
    }

    async fn package(&self, id: &str) -> Result<Option<Installed>, String> {
        Ok(niles_db::PostgresPlugins::package(self, id)
            .await?
            .map(from_row))
    }

    async fn install(&self, id: &str, version: &str, package: &Package) -> Result<(), String> {
        let source = package.source.as_ref();
        niles_db::PostgresPlugins::install(
            self,
            &niles_db::PluginInstall {
                id,
                version,
                manifest: &package.manifest,
                component: &package.component,
                logo: package.logo.as_deref(),
                repo: source.map(|s| s.repo.as_str()),
                addon: source.and_then(|s| s.addon.as_deref()),
                release_tag: source.map(|s| s.tag.as_str()),
                release_url: source.map(|s| s.release_url.as_str()),
            },
        )
        .await
    }

    async fn roll_back(&self, id: &str) -> Result<Option<Installed>, String> {
        Ok(niles_db::PostgresPlugins::roll_back(self, id)
            .await?
            .map(from_row))
    }

    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, String> {
        niles_db::PostgresPlugins::set_enabled(self, id, enabled).await
    }

    async fn set_update(&self, id: &str, update: Option<&Update>) -> Result<(), String> {
        let json = update
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| e.to_string())?;
        niles_db::PostgresPlugins::set_update(self, id, json.as_deref()).await
    }

    async fn set_notice(&self, id: &str, notice: Option<&str>) -> Result<(), String> {
        niles_db::PostgresPlugins::set_notice(self, id, notice).await
    }

    async fn count_install(&self, repo: &str) -> Result<bool, String> {
        niles_db::PostgresPlugins::count_install(self, repo).await
    }

    async fn logo(&self, id: &str) -> Result<Option<String>, String> {
        niles_db::PostgresPlugins::logo(self, id).await
    }

    async fn remove(&self, id: &str) -> Result<(), String> {
        niles_db::PostgresPlugins::remove(self, id).await
    }

    async fn get(&self, plugin: &str, key: &str) -> Result<Option<String>, String> {
        niles_db::PostgresPlugins::get(self, plugin, key).await
    }

    async fn set(&self, plugin: &str, key: &str, value: &str) -> Result<Stored, String> {
        let kept = niles_db::PostgresPlugins::set(self, plugin, key, value, STORAGE_QUOTA).await?;
        Ok(if kept { Stored::Yes } else { Stored::OverQuota })
    }

    async fn delete(&self, plugin: &str, key: &str) -> Result<(), String> {
        niles_db::PostgresPlugins::delete(self, plugin, key).await
    }

    async fn keys(&self, plugin: &str, prefix: &str) -> Result<Vec<String>, String> {
        niles_db::PostgresPlugins::keys(self, plugin, prefix).await
    }

    async fn devices(&self, plugin: &str) -> Result<Vec<KnownDevice>, String> {
        Ok(niles_db::PostgresPlugins::devices(self, plugin)
            .await?
            .into_iter()
            .filter_map(|row| {
                Some(KnownDevice {
                    class: class_from_name(&row.class)?,
                    id: row.device,
                    light: LightCapabilities {
                        color_temp: row.color_temp,
                        rgb: row.rgb,
                    },
                    controllable: row.controllable,
                    room_light: row.room_light,
                    reports_actions: row.reports_actions,
                })
            })
            .collect())
    }

    async fn put_device(&self, plugin: &str, device: &KnownDevice) -> Result<(), String> {
        let row = niles_db::PluginDeviceRow {
            device: device.id.clone(),
            class: class_name(device.class).to_string(),
            color_temp: device.light.color_temp,
            rgb: device.light.rgb,
            controllable: device.controllable,
            room_light: device.room_light,
            reports_actions: device.reports_actions,
        };
        niles_db::PostgresPlugins::put_device(self, plugin, &row).await
    }

    async fn remove_device(&self, plugin: &str, id: &str) -> Result<(), String> {
        niles_db::PostgresPlugins::remove_device(self, plugin, id).await
    }
}

/// A class as the interface spells it.
pub fn class_name(class: DeviceClass) -> &'static str {
    match class {
        DeviceClass::Light => "light",
        DeviceClass::Outlet => "outlet",
        DeviceClass::Switch => "switch",
        DeviceClass::Contact => "contact",
        DeviceClass::Motion => "motion",
        DeviceClass::Cover => "cover",
        _ => "sensor",
    }
}

fn class_from_name(name: &str) -> Option<DeviceClass> {
    Some(match name {
        "light" => DeviceClass::Light,
        "outlet" => DeviceClass::Outlet,
        "switch" => DeviceClass::Switch,
        "contact" => DeviceClass::Contact,
        "motion" => DeviceClass::Motion,
        "cover" => DeviceClass::Cover,
        "sensor" => DeviceClass::Sensor,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_quota_counts_keys_and_values_and_a_replacement_frees_its_old_value() {
        let store = MemoryStore::new();
        let half = "x".repeat(STORAGE_QUOTA / 2);
        assert_eq!(store.set("p", "a", &half).await.unwrap(), Stored::Yes);
        assert_eq!(store.set("p", "b", &half).await.unwrap(), Stored::OverQuota);
        // Replacing `a` with the same size fits: its old value goes.
        assert_eq!(store.set("p", "a", &half).await.unwrap(), Stored::Yes);
        // Another plugin has a quota of its own.
        assert_eq!(store.set("q", "b", &half).await.unwrap(), Stored::Yes);
    }

    #[tokio::test]
    async fn keys_list_in_order_by_prefix() {
        let store = MemoryStore::new();
        for key in ["day:2026-10-14", "day:2026-10-13", "cursor"] {
            store.set("p", key, "1").await.unwrap();
        }
        assert_eq!(
            store.keys("p", "day:").await.unwrap(),
            vec!["day:2026-10-13", "day:2026-10-14"]
        );
        assert_eq!(store.keys("p", "").await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_dev_store_keeps_its_storage_in_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".niles-dev").join("storage.json");
        MemoryStore::in_file(file.clone())
            .set("p", "cursor", "42")
            .await
            .unwrap();
        let again = MemoryStore::in_file(file);
        assert_eq!(
            again.get("p", "cursor").await.unwrap().as_deref(),
            Some("42")
        );
    }

    fn package(version: &str) -> Package {
        Package {
            manifest: format!("version = \"{version}\""),
            component: version.as_bytes().to_vec(),
            logo: None,
            source: None,
        }
    }

    async fn notice(store: &MemoryStore) -> Option<String> {
        store.records().await.unwrap()[0].notice.clone()
    }

    #[tokio::test]
    async fn a_rollback_notice_is_kept_until_the_next_update_or_rollback() {
        let store = MemoryStore::new();
        store
            .install("p", "0.1.0", &package("0.1.0"))
            .await
            .unwrap();
        store
            .install("p", "0.2.0", &package("0.2.0"))
            .await
            .unwrap();
        store.roll_back("p").await.unwrap();
        store
            .set_notice("p", Some("Version 0.2.0 failed"))
            .await
            .unwrap();
        assert_eq!(
            notice(&store).await.as_deref(),
            Some("Version 0.2.0 failed")
        );
        store.set_enabled("p", false).await.unwrap();
        assert!(
            notice(&store).await.is_some(),
            "turning it off says nothing new"
        );

        store
            .install("p", "0.3.0", &package("0.3.0"))
            .await
            .unwrap();
        assert_eq!(notice(&store).await, None, "an update starts afresh");
        store.set_notice("p", Some("again")).await.unwrap();
        store.roll_back("p").await.unwrap();
        assert_eq!(notice(&store).await, None, "and so does a rollback");
        store.set_notice("nobody", Some("x")).await.unwrap();
    }

    #[test]
    fn classes_round_trip_by_name() {
        for class in [
            DeviceClass::Light,
            DeviceClass::Outlet,
            DeviceClass::Switch,
            DeviceClass::Contact,
            DeviceClass::Motion,
            DeviceClass::Cover,
            DeviceClass::Sensor,
        ] {
            assert_eq!(class_from_name(class_name(class)), Some(class));
        }
    }

    #[tokio::test]
    async fn an_install_is_counted_once_and_removal_keeps_the_count() {
        let store = MemoryStore::new();
        assert!(store.count_install("jane/hello").await.unwrap());
        assert!(
            !store.count_install("jane/hello").await.unwrap(),
            "ever after, even once the plugin is removed"
        );
        store
            .install("hello", "0.1.0", &package("0.1.0"))
            .await
            .unwrap();
        store.remove("hello").await.unwrap();
        assert!(
            !store.count_install("jane/hello").await.unwrap(),
            "installing it again is not counted again"
        );
    }
}
