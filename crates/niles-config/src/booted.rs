//! What `niles serve` started with, of everything it reads only once.
//!
//! Whether a restart waits is answered by comparing this with the store
//! now, never by remembering writes: a change from the app, by voice or
//! over the API counts the same, changing it back clears it, and every
//! session asking gets the same answer.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Config, Reload, SECTIONS, section_reload};

/// What `niles serve` started with, of everything it reads only once.
///
/// See the module doc for why the comparison, not a remembered list.
#[derive(Debug, Clone)]
pub struct Booted {
    /// Each boot section's effective value, without the settings in it
    /// that are re-read and with empty tables dropped. A section the
    /// table did not have is absent.
    sections: BTreeMap<String, toml::Value>,
    /// A digest of what each long-lived client was built with, by the
    /// section it belongs to. Never the credential itself, so nothing
    /// holding a `Booted` can print one.
    credentials: Vec<(&'static str, Vec<u8>)>,
}

impl Booted {
    /// From the table and config the startup sequence builds from,
    /// once the stored secrets are loaded: the clients are built after
    /// that, from the same snapshot.
    pub fn new(effective: &toml::Table, config: &Config) -> Self {
        let mut sections = BTreeMap::new();
        for key in effective.keys() {
            if section_reload(key) == Reload::Boot
                && let Some(value) = comparable(key, effective.get(key))
            {
                sections.insert(key.clone(), value);
            }
        }
        Self {
            sections,
            credentials: credentials(config),
        }
    }

    /// The sections a restart would apply a change to now, in
    /// `SECTIONS` order, then any other by name.
    #[must_use]
    pub fn differs(&self, effective: &toml::Table, config: &Config) -> Vec<String> {
        let mut names: BTreeSet<&str> = self.sections.keys().map(String::as_str).collect();
        let boot_in_effective: Vec<&str> = effective
            .keys()
            .map(String::as_str)
            .filter(|key| section_reload(key) == Reload::Boot)
            .collect();
        names.extend(boot_in_effective);

        let mut differs: Vec<String> = names
            .into_iter()
            .filter(|name| {
                comparable(name, effective.get(*name)).as_ref() != self.sections.get(*name)
            })
            .map(str::to_string)
            .collect();

        // Credentials resolve over the live config and secrets, so a
        // password saved after start counts even though no section's
        // table changed.
        for ((was_section, was_digest), (now_section, now_digest)) in
            self.credentials.iter().zip(credentials(config))
        {
            debug_assert_eq!(*was_section, now_section, "the digest list is fixed");
            if was_digest != &now_digest && !differs.iter().any(|d| d == now_section) {
                differs.push((*now_section).to_string());
            }
        }

        differs.sort_by_key(|name| {
            (
                SECTIONS
                    .iter()
                    .position(|s| s == name)
                    .unwrap_or(usize::MAX),
                name.clone(),
            )
        });
        differs
    }
}

/// The comparable form of a section's value: without the settings in it
/// that are re-read, and with empty tables dropped so "reset left an
/// empty `[mqtt]` table" compares equal to "absent".
fn comparable(section: &str, value: Option<&toml::Value>) -> Option<toml::Value> {
    let mut value = value?.clone();
    if let toml::Value::Table(table) = &mut value {
        for live in crate::live_in(section) {
            remove_path(table, live);
        }
        prune_empty(table);
        if table.is_empty() {
            return None;
        }
    }
    Some(value)
}

/// Remove a dotted path from a table. A missing path is not an error:
/// the live setting may never have been set.
fn remove_path(table: &mut toml::Table, path: &str) {
    let Some((head, rest)) = path.split_once('.') else {
        table.remove(path);
        return;
    };
    if let Some(toml::Value::Table(inner)) = table.get_mut(head) {
        remove_path(inner, rest);
    }
}

/// Drop sub-tables that are empty, bottom-up, so an emptied branch
/// disappears with its children.
fn prune_empty(table: &mut toml::Table) {
    let keys: Vec<String> = table.keys().cloned().collect();
    for key in keys {
        if let Some(toml::Value::Table(inner)) = table.get_mut(&key) {
            prune_empty(inner);
            if inner.is_empty() {
                table.remove(&key);
            }
        }
    }
}

/// A digest of what each long-lived client was built with, by the
/// section it belongs to.
///
/// Never the value: a digest, so nothing holding a `Booted` can print a
/// credential.
fn credentials(config: &Config) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "mqtt",
            digest(
                config
                    .mqtt
                    .resolve_credentials()
                    .map(|(user, pass)| vec![user, pass]),
            ),
        ),
        ("llm", digest(llm_parts(config))),
    ]
}

/// Tier 1's endpoint and key, then Tier 2's when it has one: what the
/// clients below `niles serve` are built from.
fn llm_parts(config: &Config) -> crate::Result<Vec<String>> {
    let tier1 = config.llm_endpoint()?;
    let mut parts = vec![tier1.base_url, tier1.api_key];
    if let Some(tier2) = config.tier2_endpoint() {
        let tier2 = tier2?;
        parts.push(tier2.base_url);
        parts.push(tier2.api_key);
    }
    Ok(parts)
}

/// Hash the parts length-first so no concatenation is ambiguous.
/// Resolution errors digest to a fixed marker: "unresolved at boot and
/// still unresolved" is equal, while "unresolved → resolved" differs,
/// which is right, because the client was built without it.
fn digest(parts: crate::Result<Vec<String>>) -> Vec<u8> {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    match parts {
        Ok(parts) => {
            for part in parts {
                hasher.update(part.len().to_le_bytes());
                hasher.update(part.as_bytes());
            }
        }
        Err(_) => hasher.update(b"unresolved"),
    }
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// `differs` reads the global secrets map, so every test here holds
    /// the guard and starts from a known-empty map.
    fn hold_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::secrets::TEST_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn store_and_booted(
        base: &str,
    ) -> (
        std::sync::MutexGuard<'static, ()>,
        crate::ConfigStore,
        Booted,
    ) {
        let guard = hold_guard();
        crate::secrets::load(HashMap::new());
        let store = crate::ConfigStore::from_str_in_memory(base).expect("fixture config is valid");
        let (config, table) = store.current_and_table();
        let booted = Booted::new(&table, &config);
        (guard, store, booted)
    }

    fn differs(store: &crate::ConfigStore, booted: &Booted) -> Vec<String> {
        let _guard = hold_guard();
        let (config, table) = store.current_and_table();
        booted.differs(&table, &config)
    }

    fn patch(s: &str) -> toml::Table {
        toml::from_str(s).expect("test patch parses")
    }

    #[test]
    fn nothing_waits_at_first() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        assert!(differs(&store, &booted).is_empty());
    }

    // The guard is held across the store's awaits on purpose: it
    // serializes the global test secrets map, and it is the test
    // mutex, never one a request waits on.
    #[expect(clippy::await_holding_lock, reason = "serializes the test secrets map")]
    #[tokio::test]
    async fn a_boot_section_changed_from_the_app_waits() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        store
            .apply(
                &patch("[api]\nbind_address = \"0.0.0.0:9999\""),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        assert_eq!(differs(&store, &booted), ["api"]);
    }

    // The guard is held across the store's awaits on purpose: it
    // serializes the global test secrets map, and it is the test
    // mutex, never one a request waits on.
    #[expect(clippy::await_holding_lock, reason = "serializes the test secrets map")]
    #[tokio::test]
    async fn a_change_by_voice_counts_the_same() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        store
            .apply(
                &patch("[api]\nbind_address = \"0.0.0.0:9999\""),
                crate::ChangeSource::Voice,
            )
            .await
            .expect("voice may change a boot section that is not auth");
        assert_eq!(differs(&store, &booted), ["api"]);
        store
            .undo(crate::ChangeSource::Voice)
            .await
            .expect("undo works")
            .expect("there is something to undo");
        assert!(differs(&store, &booted).is_empty());
    }

    // The guard is held across the store's awaits on purpose: it
    // serializes the global test secrets map, and it is the test
    // mutex, never one a request waits on.
    #[expect(clippy::await_holding_lock, reason = "serializes the test secrets map")]
    #[tokio::test]
    async fn changing_it_back_clears_it() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        store
            .apply(
                &patch("[api]\nbind_address = \"0.0.0.0:9999\""),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        store
            .apply(
                &patch("[api]\nbind_address = \"0.0.0.0:8080\""),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        assert!(differs(&store, &booted).is_empty());
    }

    // The guard is held across the store's awaits on purpose: it
    // serializes the global test secrets map, and it is the test
    // mutex, never one a request waits on.
    #[expect(clippy::await_holding_lock, reason = "serializes the test secrets map")]
    #[tokio::test]
    async fn resetting_it_clears_it() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        store
            .apply(
                &patch("[api]\nbind_address = \"0.0.0.0:9999\""),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        store
            .reset("api.bind_address", crate::ChangeSource::Api)
            .await
            .expect("reset works");
        assert!(differs(&store, &booted).is_empty());
    }

    // The guard is held across the store's awaits on purpose: it
    // serializes the global test secrets map, and it is the test
    // mutex, never one a request waits on.
    #[expect(clippy::await_holding_lock, reason = "serializes the test secrets map")]
    #[tokio::test]
    async fn a_reset_that_leaves_an_empty_table_clears_it() {
        // A base with no `[mqtt]` table at all.
        let base = r#"
[home]
name = "test home"
latitude = 56.1572
longitude = 10.2107
timezone = "Europe/Copenhagen"

[api]
bind_address = "0.0.0.0:8080"
"#;
        let (_guard, store, booted) = store_and_booted(base);
        store
            .apply(
                &patch("[mqtt]\nclient_id = \"tester\""),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        assert_eq!(differs(&store, &booted), ["mqtt"]);
        store
            .reset("mqtt.client_id", crate::ChangeSource::Api)
            .await
            .expect("reset works");
        assert!(
            differs(&store, &booted).is_empty(),
            "an empty [mqtt] left by the reset compares equal to absent"
        );
    }

    // The guard is held across the store's awaits on purpose: it
    // serializes the global test secrets map, and it is the test
    // mutex, never one a request waits on.
    #[expect(clippy::await_holding_lock, reason = "serializes the test secrets map")]
    #[tokio::test]
    async fn a_hot_section_never_waits() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        store
            .apply(
                &patch("[lighting]\ndaytime_brightness = 85"),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        assert!(differs(&store, &booted).is_empty());
    }

    // The guard is held across the store's awaits on purpose: it
    // serializes the global test secrets map, and it is the test
    // mutex, never one a request waits on.
    #[expect(clippy::await_holding_lock, reason = "serializes the test secrets map")]
    #[tokio::test]
    async fn the_voice_lock_never_waits() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        store
            .apply(
                &patch("[recognition]\nknown_voices_only = true"),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        assert!(
            differs(&store, &booted).is_empty(),
            "the voice lock is re-read per turn"
        );
    }

    // The guard is held across the store's awaits on purpose: it
    // serializes the global test secrets map, and it is the test
    // mutex, never one a request waits on.
    #[expect(clippy::await_holding_lock, reason = "serializes the test secrets map")]
    #[tokio::test]
    async fn two_boot_sections_are_listed_in_section_order() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        store
            .apply(
                &patch("[api]\nbind_address = \"0.0.0.0:9999\""),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        store
            .apply(
                &patch("[mqtt]\nclient_id = \"tester\""),
                crate::ChangeSource::Api,
            )
            .await
            .expect("apply works");
        assert_eq!(differs(&store, &booted), ["mqtt", "api"], "SECTIONS order");
    }

    #[test]
    fn a_stored_broker_password_saved_after_start_waits() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        let mut stored = HashMap::new();
        stored.insert("mqtt.password".to_string(), "x".to_string());
        stored.insert("mqtt.username".to_string(), "u".to_string());
        crate::secrets::load(stored);
        assert_eq!(differs(&store, &booted), ["mqtt"]);
        crate::secrets::load(HashMap::new());
        assert!(differs(&store, &booted).is_empty());
    }

    #[test]
    fn a_language_model_key_saved_after_start_waits() {
        let (_guard, store, booted) = store_and_booted(crate::tests_support::valid_toml());
        let mut stored = HashMap::new();
        stored.insert("provider.cerebras.api_key".to_string(), "k".to_string());
        crate::secrets::load(stored);
        assert_eq!(differs(&store, &booted), ["llm"]);
        crate::secrets::load(HashMap::new());
        assert!(differs(&store, &booted).is_empty());
    }

    #[test]
    fn a_password_from_the_environment_wins() {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var("NILES_TEST_BOOTED_MQTT_USERNAME", "env-user");
            std::env::set_var("NILES_TEST_BOOTED_MQTT_PASSWORD", "env-pass");
        }
        let base = crate::tests_support::valid_toml()
            .replace(
                "username_env = \"NILES_MQTT_USERNAME\"",
                "username_env = \"NILES_TEST_BOOTED_MQTT_USERNAME\"",
            )
            .replace(
                "password_env = \"NILES_MQTT_PASSWORD\"",
                "password_env = \"NILES_TEST_BOOTED_MQTT_PASSWORD\"",
            );
        let (_guard, store, booted) = store_and_booted(&base);
        // Stored values differ from the environment, but the environment
        // wins both now and after a restart, so nothing waits.
        let mut stored = HashMap::new();
        stored.insert("mqtt.password".to_string(), "other".to_string());
        stored.insert("mqtt.username".to_string(), "other".to_string());
        crate::secrets::load(stored);
        assert!(
            differs(&store, &booted).is_empty(),
            "the environment wins over stored values, now and after a restart"
        );
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::remove_var("NILES_TEST_BOOTED_MQTT_USERNAME");
            std::env::remove_var("NILES_TEST_BOOTED_MQTT_PASSWORD");
        }
    }
}
