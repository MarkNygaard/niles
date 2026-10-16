//! The credentials a plugin keeps itself (`niles:integration/secrets`,
//! interface 0.3): an OAuth token it refreshed, under
//! `plugins.<id>.kept.<key>` in Niles's encrypted secret store.
//!
//! [`KeptSecrets`] is that store as the runtime sees it, by full key.
//! Its `set` must be atomic: the value is replaced whole or not at all,
//! and an error leaves the previous one readable. `serve` gives it the
//! Postgres secret store, which seals the value and writes it in one
//! statement; without a database, and for `niles plugin dev`, it is
//! [`MemoryKept`].

use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The most keys one plugin keeps.
pub const MAX_KEPT: usize = 16;
/// The longest key, in bytes.
pub const MAX_KEPT_KEY: usize = 64;
/// The largest value, in bytes.
pub const MAX_KEPT_VALUE: usize = 4 * 1024;

/// Where `plugin`'s kept secret `key` lives in the secret store.
pub fn full_key(plugin: &str, key: &str) -> String {
    format!("{}{key}", prefix(plugin))
}

/// What every one of `plugin`'s kept secrets starts with.
pub fn prefix(plugin: &str) -> String {
    format!("plugins.{plugin}.kept.")
}

#[async_trait]
pub trait KeptSecrets: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<String>, String>;
    /// Replace the value whole, or change nothing and say why.
    async fn set(&self, key: &str, value: &str) -> Result<(), String>;
    async fn delete(&self, key: &str) -> Result<(), String>;
    /// The keys starting with `prefix`.
    async fn keys(&self, prefix: &str) -> Result<Vec<String>, String>;
}

/// In memory: gone when Niles stops.
#[derive(Default)]
pub struct MemoryKept {
    values: Mutex<BTreeMap<String, String>>,
}

impl MemoryKept {
    pub fn new() -> Self {
        Self::default()
    }

    fn values(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
        self.values.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl KeptSecrets for MemoryKept {
    async fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.values().get(key).cloned())
    }

    async fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.values().insert(key.to_string(), value.to_string());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        self.values().remove(key);
        Ok(())
    }

    async fn keys(&self, prefix: &str) -> Result<Vec<String>, String> {
        Ok(self
            .values()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}
