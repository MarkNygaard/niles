//! Where installed plugins, their private storage, and the devices Niles
//! holds for them are kept.
//!
//! Postgres when Niles has a database ([`niles_db::PostgresPlugins`]);
//! in memory otherwise, and for `niles plugin dev`, which can keep the
//! storage in a file beside the plugin so a restart keeps it.

use async_trait::async_trait;
use niles_core::{DeviceClass, LightCapabilities};
use std::collections::BTreeMap;
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

/// A device Niles holds for a plugin, so it outlives the instance.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownDevice {
    /// `room/name`, as the plugin names it.
    pub id: String,
    pub class: DeviceClass,
    pub light: LightCapabilities,
    pub controllable: bool,
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
    /// Keep a plugin, replacing one with the same id. Internal: what an
    /// approved install writes, after the caller has checked it.
    async fn install(&self, id: &str, version: &str, manifest: &str, component: &[u8])
    -> Result<(), String>;

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
    installed: BTreeMap<String, Installed>,
    storage: BTreeMap<String, BTreeMap<String, String>>,
    devices: BTreeMap<String, BTreeMap<String, KnownDevice>>,
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
        Ok(self.lock().installed.values().cloned().collect())
    }

    async fn install(
        &self,
        id: &str,
        _version: &str,
        manifest: &str,
        component: &[u8],
    ) -> Result<(), String> {
        self.lock().installed.insert(
            id.to_string(),
            Installed {
                manifest: manifest.to_string(),
                component: component.to_vec(),
            },
        );
        Ok(())
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

#[async_trait]
impl PluginStore for niles_db::PostgresPlugins {
    async fn installed(&self) -> Result<Vec<Installed>, String> {
        Ok(self
            .enabled()
            .await?
            .into_iter()
            .map(|row| Installed {
                manifest: row.manifest,
                component: row.component,
            })
            .collect())
    }

    async fn install(
        &self,
        id: &str,
        version: &str,
        manifest: &str,
        component: &[u8],
    ) -> Result<(), String> {
        niles_db::PostgresPlugins::install(self, id, version, manifest, None, component).await
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
        assert_eq!(again.get("p", "cursor").await.unwrap().as_deref(), Some("42"));
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
}
