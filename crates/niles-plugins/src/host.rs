//! What each import does: the Rust behind `niles:integration`'s
//! interfaces, for one instance of one plugin.
//!
//! [`HostState`] is the data of the instance's wasmtime `Store`. Every
//! function checks the manifest's permissions itself, so a component
//! that imports an interface it was not granted — which every
//! JavaScript and Python one does, since their toolchains import the
//! whole world — gets `not-permitted` rather than a link error.

use crate::bindings::niles::integration::{
    config, devices, events, http as wit_http, log, notify, storage, timers, types, ui,
};
use crate::manifest::{EventPermission, Notify, SettingKind, SettingValue, TAKEN_READINGS};
use crate::policy::HttpPolicy;
use crate::runtime::{House, Plugin};
use crate::store::{KnownDevice, MAX_KEY, Stored};
use niles_core::{
    Device, DeviceClass, DeviceId, DeviceName, DeviceState, Event, LightCapabilities, Reading,
    RoomName, validate_reading_name,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use types::HostError;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

pub const MAX_DEVICES: usize = 256;
pub const MAX_TIMERS: usize = 32;
pub const MAX_READINGS: usize = 32;
pub const NOTIFICATIONS_PER_HOUR: usize = 6;
const MAX_ACTION: usize = 64;
const MAX_MESSAGE: usize = 1000;
const MAX_PATTERNS: usize = 64;

/// A timer an instance set.
#[derive(Debug, Clone)]
pub(crate) struct Timer {
    pub every: Option<Duration>,
    pub due: Instant,
}

/// The data of one instance's `Store`.
pub(crate) struct HostState {
    wasi: WasiCtx,
    http_ctx: WasiHttpCtx,
    table: ResourceTable,
    pub memory: MemoryCap,
    pub plugin: Arc<Plugin>,
    pub house: Arc<House>,
    /// The settings as they were when the instance started. Saving
    /// settings starts a new instance, so these never change under it.
    settings: toml::Table,
    policy: HttpPolicy,
    /// When the call in progress has to be finished. Host calls that
    /// wait — HTTP, the database — never wait past it.
    pub deadline: Instant,
    pub timers: BTreeMap<String, Timer>,
}

impl HostState {
    pub fn new(
        plugin: Arc<Plugin>,
        house: Arc<House>,
        settings: toml::Table,
        policy: HttpPolicy,
    ) -> Self {
        let mut wasi = wasmtime_wasi::WasiCtxBuilder::new();
        // No directories, no environment, no arguments: the builder's
        // defaults. Sockets refused outright, name lookups too. Output
        // goes to the plugin's log.
        wasi.allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false)
            .socket_addr_check(|_, _| Box::pin(async { false }))
            .stdout(crate::output::LogStream::new(
                plugin.clone(),
                log::Level::Info,
            ))
            .stderr(crate::output::LogStream::new(
                plugin.clone(),
                log::Level::Warn,
            ));
        let memory = MemoryCap::new(plugin.manifest.memory_mb());
        Self {
            wasi: wasi.build(),
            http_ctx: WasiHttpCtx::new(),
            table: ResourceTable::new(),
            memory,
            plugin,
            house,
            settings,
            policy,
            deadline: Instant::now(),
            timers: BTreeMap::new(),
        }
    }

    /// Where this instance may send HTTP, which also decides where a
    /// page's images may come from.
    pub fn policy(&self) -> &HttpPolicy {
        &self.policy
    }

    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Wait for `fut`, but never past the call's deadline.
    fn wait<T>(&self, fut: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        let left = self.remaining();
        self.house
            .handle
            .block_on(async { tokio::time::timeout(left, fut).await })
            .unwrap_or_else(|_| Err("it took longer than the call has left".into()))
    }

    fn id(&self) -> &str {
        &self.plugin.id
    }

    fn device_id(&self, id: &str) -> Result<DeviceId, HostError> {
        let invalid = |why: String| HostError::Invalid(format!("device id {id:?}: {why}"));
        let (room, name) = id
            .split_once('/')
            .ok_or_else(|| invalid("a device id is room/name".into()))?;
        let room = RoomName::parse(room).map_err(|e| invalid(e.to_string()))?;
        let name = DeviceName::parse(name).map_err(|e| invalid(e.to_string()))?;
        DeviceId::new(self.plugin.source.clone(), room, name).map_err(|e| invalid(e.to_string()))
    }

    fn own_device(&self, id: &str) -> Result<DeviceId, HostError> {
        self.may_use_devices()?;
        let device = self.device_id(id)?;
        if !self.plugin.devices().contains_key(&device) {
            return Err(HostError::Invalid(format!(
                "{id} is not one of this plugin's devices; upsert it first"
            )));
        }
        Ok(device)
    }

    fn may_use_devices(&self) -> Result<(), HostError> {
        if self.plugin.manifest.permissions.devices {
            Ok(())
        } else {
            Err(HostError::NotPermitted(
                "the manifest does not ask for devices".into(),
            ))
        }
    }

    fn publish(&self, event: Event) {
        self.house.bus.publish(event);
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for HostState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http_ctx,
            table: &mut self.table,
            hooks: wasmtime_wasi_http::default_hooks(),
        }
    }
}

/// The memory cap, over every linear memory the component has rather
/// than each one: a JavaScript or Python component can have several.
pub(crate) struct MemoryCap {
    limit: usize,
    used: usize,
}

impl MemoryCap {
    fn new(mb: u32) -> Self {
        Self {
            limit: mb as usize * 1024 * 1024,
            used: 0,
        }
    }
}

impl wasmtime::ResourceLimiter for MemoryCap {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let after = self.used - current.min(self.used) + desired;
        if after > self.limit {
            return Err(wasmtime::format_err!(
                "out of memory: the plugin may use {} MiB",
                self.limit / (1024 * 1024)
            ));
        }
        self.used = after;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 100_000)
    }
}

impl types::Host for HostState {}

impl log::Host for HostState {
    fn log(&mut self, level: log::Level, message: String) {
        self.plugin.log(level, &message);
    }
}

impl config::Host for HostState {
    fn get(&mut self, key: String) -> Option<config::SettingValue> {
        let Some(setting) = self.plugin.manifest.setting(&key) else {
            self.plugin.log(
                log::Level::Warn,
                &format!("config.get({key:?}): the manifest declares no such setting"),
            );
            return None;
        };
        Some(match setting.value(&self.settings)? {
            SettingValue::Text(t) => config::SettingValue::Text(t),
            SettingValue::Number(n) => config::SettingValue::Number(n),
            SettingValue::Boolean(b) => config::SettingValue::Boolean(b),
        })
    }

    fn secret(&mut self, key: String) -> Option<String> {
        match self.plugin.manifest.setting(&key) {
            Some(s) if s.kind == SettingKind::Secret => (self.house.secrets)(self.id(), &key),
            Some(_) => None,
            None => {
                self.plugin.log(
                    log::Level::Warn,
                    &format!("config.secret({key:?}): the manifest declares no such setting"),
                );
                None
            }
        }
    }
}

fn check_key(key: &str) -> Result<(), HostError> {
    if key.is_empty() || key.len() > MAX_KEY {
        return Err(HostError::Invalid(format!(
            "a storage key is 1 to {MAX_KEY} bytes"
        )));
    }
    Ok(())
}

impl storage::Host for HostState {
    fn get(&mut self, key: String) -> Option<String> {
        check_key(&key).ok()?;
        let store = self.house.store.clone();
        let id = self.id().to_string();
        self.wait(async { store.get(&id, &key).await })
            .unwrap_or_else(|e| {
                tracing::warn!(plugin = %id, "storage.get failed: {e}");
                None
            })
    }

    fn set(&mut self, key: String, value: String) -> Result<(), HostError> {
        check_key(&key)?;
        if key.contains('\0') || value.contains('\0') {
            return Err(HostError::Invalid(
                "storage cannot hold a NUL character".into(),
            ));
        }
        let store = self.house.store.clone();
        let id = self.id().to_string();
        match self.wait(async { store.set(&id, &key, &value).await }) {
            Ok(Stored::Yes) => Ok(()),
            Ok(Stored::OverQuota) => Err(HostError::Limit(
                "storage is full: all keys and values together are at most 1 MiB".into(),
            )),
            Err(e) => Err(HostError::Unavailable(e)),
        }
    }

    fn delete(&mut self, key: String) {
        let store = self.house.store.clone();
        let id = self.id().to_string();
        if let Err(e) = self.wait(async { store.delete(&id, &key).await }) {
            tracing::warn!(plugin = %id, "storage.delete failed: {e}");
        }
    }

    fn list_keys(&mut self, prefix: String) -> Vec<String> {
        let store = self.house.store.clone();
        let id = self.id().to_string();
        self.wait(async { store.keys(&id, &prefix).await })
            .unwrap_or_else(|e| {
                tracing::warn!(plugin = %id, "storage.list-keys failed: {e}");
                Vec::new()
            })
    }
}

impl wit_http::Host for HostState {
    fn send(&mut self, req: wit_http::Request) -> Result<wit_http::Response, wit_http::HttpError> {
        if self.policy.is_empty() {
            return Err(wit_http::HttpError::NotPermitted(
                "the manifest declares no http or local_network hosts".into(),
            ));
        }
        let method = match req.method {
            wit_http::Method::Get => reqwest::Method::GET,
            wit_http::Method::Head => reqwest::Method::HEAD,
            wit_http::Method::Post => reqwest::Method::POST,
            wit_http::Method::Put => reqwest::Method::PUT,
            wit_http::Method::Patch => reqwest::Method::PATCH,
            wit_http::Method::Delete => reqwest::Method::DELETE,
        };
        let request = crate::http::Request {
            method,
            url: req.url,
            headers: req.headers,
            body: req.body,
            timeout: req
                .timeout_ms
                .map(|ms| Duration::from_millis(u64::from(ms))),
        };
        let user_agent = format!(
            "niles-plugin/{}/{}",
            self.plugin.id, self.plugin.manifest.version
        );
        let deadline = self.deadline;
        let policy = self.policy.clone();
        let sent =
            self.house
                .handle
                .block_on(crate::http::send(&policy, &user_agent, request, deadline));
        match sent {
            Ok(r) => Ok(wit_http::Response {
                status: r.status,
                headers: r.headers,
                body: r.body,
            }),
            Err(failure) => {
                use crate::http::Failure;
                if let Failure::NotPermitted(why) = &failure {
                    self.plugin
                        .log(log::Level::Warn, &format!("http refused: {why}"));
                }
                Err(match failure {
                    Failure::NotPermitted(why) => wit_http::HttpError::NotPermitted(why),
                    Failure::InvalidRequest(why) => wit_http::HttpError::InvalidRequest(why),
                    Failure::Timeout => wit_http::HttpError::Timeout,
                    Failure::Connection(why) => wit_http::HttpError::Connection(why),
                    Failure::TooLarge => wit_http::HttpError::TooLarge,
                })
            }
        }
    }
}

/// The interface's class, as Niles's.
pub(crate) fn class_from_wit(class: types::DeviceClass) -> DeviceClass {
    match class {
        types::DeviceClass::Light => DeviceClass::Light,
        types::DeviceClass::Outlet => DeviceClass::Outlet,
        types::DeviceClass::Switch => DeviceClass::Switch,
        types::DeviceClass::Contact => DeviceClass::Contact,
        types::DeviceClass::Motion => DeviceClass::Motion,
        types::DeviceClass::Cover => DeviceClass::Cover,
        types::DeviceClass::Sensor => DeviceClass::Sensor,
    }
}

/// Niles's class, as the interface's. A class the interface has no word
/// for is a sensor: something that reports.
pub(crate) fn class_to_wit(class: DeviceClass) -> types::DeviceClass {
    match class {
        DeviceClass::Light => types::DeviceClass::Light,
        DeviceClass::Outlet => types::DeviceClass::Outlet,
        DeviceClass::Switch => types::DeviceClass::Switch,
        DeviceClass::Contact => types::DeviceClass::Contact,
        DeviceClass::Motion => types::DeviceClass::Motion,
        DeviceClass::Cover => types::DeviceClass::Cover,
        _ => types::DeviceClass::Sensor,
    }
}

/// A state a plugin reported, checked against the units Niles promises.
pub(crate) fn state_from_wit(s: types::DeviceState) -> Result<DeviceState, HostError> {
    let invalid = |why: &str| Err(HostError::Invalid(why.to_string()));
    let percent = |v: Option<u8>| v.is_none_or(|v| v <= 100);
    if !percent(s.brightness) || !percent(s.battery_percent) || !percent(s.position) {
        return invalid("brightness, battery and position are 0-100");
    }
    let finite = |v: Option<f32>| v.is_none_or(f32::is_finite);
    if !finite(s.temperature_celsius) || !finite(s.humidity_percent) || !finite(s.illuminance_lux) {
        return invalid("a measurement is not a number");
    }
    if s.readings.len() > MAX_READINGS {
        return Err(HostError::Limit(format!(
            "at most {MAX_READINGS} readings per device"
        )));
    }
    let mut readings = BTreeMap::new();
    for r in s.readings {
        if validate_reading_name(&r.name).is_err() {
            return Err(HostError::Invalid(format!(
                "reading {:?}: names are [a-z0-9_]+",
                r.name
            )));
        }
        if TAKEN_READINGS.contains(&r.name.as_str()) {
            return Err(HostError::Invalid(format!(
                "reading {:?} has a field of its own in the device state",
                r.name
            )));
        }
        if !r.value.is_finite() {
            return Err(HostError::Invalid(format!(
                "reading {:?} is not a number",
                r.name
            )));
        }
        readings.insert(r.name, Reading::new(r.value, r.unit));
    }
    Ok(DeviceState {
        on: s.on,
        brightness: s.brightness,
        color_temp_kelvin: s.color_temp_kelvin,
        rgb: s.rgb.map(|c| [c.r, c.g, c.b]),
        temperature_celsius: s.temperature_celsius,
        humidity_percent: s.humidity_percent,
        battery_percent: s.battery_percent,
        open: s.open,
        motion: s.motion,
        illuminance_lux: s.illuminance_lux,
        position: s.position,
        readings,
    })
}

pub(crate) fn state_to_wit(s: &DeviceState) -> types::DeviceState {
    types::DeviceState {
        on: s.on,
        brightness: s.brightness,
        color_temp_kelvin: s.color_temp_kelvin,
        rgb: s.rgb.map(|[r, g, b]| types::RgbColor { r, g, b }),
        temperature_celsius: s.temperature_celsius,
        humidity_percent: s.humidity_percent,
        battery_percent: s.battery_percent,
        open: s.open,
        motion: s.motion,
        illuminance_lux: s.illuminance_lux,
        position: s.position,
        readings: s
            .readings
            .iter()
            .map(|(name, r)| types::Reading {
                name: name.clone(),
                value: r.value,
                unit: r.unit.clone(),
            })
            .collect(),
    }
}

fn is_empty(state: &DeviceState) -> bool {
    *state == DeviceState::default()
}

impl devices::Host for HostState {
    fn upsert(&mut self, device: types::Device) -> Result<(), HostError> {
        self.may_use_devices()?;
        let id = self.device_id(&device.id)?;
        let known = self.plugin.devices();
        if !known.contains_key(&id) && known.len() >= MAX_DEVICES {
            return Err(HostError::Limit(format!(
                "at most {MAX_DEVICES} devices per plugin"
            )));
        }
        let previous = known.get(&id).cloned();
        drop(known);

        let class = class_from_wit(device.class);
        let light = if class == DeviceClass::Light {
            LightCapabilities {
                color_temp: device.light.color_temp,
                rgb: device.light.rgb,
            }
        } else {
            LightCapabilities::default()
        };
        let partial = state_from_wit(device.state)?;
        let meta = KnownDevice {
            id: device.id.clone(),
            class,
            light,
            controllable: device.controllable,
        };

        let registry = &self.house.registry;
        let existing = registry.get(&id);
        // Class, capabilities and availability come from this call; the
        // state is what Niles already knew, merged with this one's.
        let replacement = Device::new(
            id.clone(),
            existing
                .as_ref()
                .map(|d| d.state.clone())
                .unwrap_or_default(),
            class,
        )
        .with_capabilities(light);
        registry.upsert(replacement);
        if !is_empty(&partial) {
            registry.merge_state(&id, partial.clone());
        }
        match &existing {
            None => {
                if let Some(device) = registry.get(&id) {
                    self.publish(Event::DeviceAdded { device });
                }
            }
            Some(before) if !before.available || !is_empty(&partial) => {
                self.publish(Event::DeviceStateChanged {
                    id: id.clone(),
                    state: if before.available {
                        partial
                    } else {
                        registry.get(&id).map(|d| d.state).unwrap_or_default()
                    },
                });
            }
            Some(_) => {}
        }

        if previous.as_ref() != Some(&meta) {
            self.plugin.devices_mut().insert(id, meta.clone());
            let store = self.house.store.clone();
            let plugin = self.id().to_string();
            if let Err(e) = self.wait(async { store.put_device(&plugin, &meta).await }) {
                tracing::warn!(plugin = %plugin, "could not remember {}: {e}", meta.id);
            }
        }
        Ok(())
    }

    fn report_state(&mut self, id: String, state: types::DeviceState) -> Result<(), HostError> {
        let device = self.own_device(&id)?;
        let partial = state_from_wit(state)?;
        let registry = &self.house.registry;
        if let Some(current) = registry.get(&device) {
            let names: std::collections::BTreeSet<&String> = current
                .state
                .readings
                .keys()
                .chain(partial.readings.keys())
                .collect();
            if names.len() > MAX_READINGS {
                return Err(HostError::Limit(format!(
                    "at most {MAX_READINGS} readings per device"
                )));
            }
        }
        if is_empty(&partial) {
            return Ok(());
        }
        registry.merge_state(&device, partial.clone());
        self.plugin.publish_state(device, partial, &self.house);
        Ok(())
    }

    fn report_action(&mut self, id: String, action: String) -> Result<(), HostError> {
        let device = self.own_device(&id)?;
        if self.plugin.devices().get(&device).map(|d| d.class) != Some(DeviceClass::Switch) {
            return Err(HostError::Invalid(format!(
                "{id} is not a switch; only switches report actions"
            )));
        }
        if action.is_empty() || action.len() > MAX_ACTION {
            return Err(HostError::Invalid(format!(
                "an action is 1 to {MAX_ACTION} bytes, such as single or double"
            )));
        }
        self.publish(Event::DeviceAction { id: device, action });
        Ok(())
    }

    fn set_available(&mut self, id: String, available: bool) -> Result<(), HostError> {
        let device = self.own_device(&id)?;
        if self.house.registry.set_available(&device, available) {
            let state = self
                .house
                .registry
                .get(&device)
                .map(|d| d.state)
                .unwrap_or_default();
            self.publish(Event::DeviceStateChanged { id: device, state });
        }
        Ok(())
    }

    fn remove(&mut self, id: String) -> Result<(), HostError> {
        let device = self.own_device(&id)?;
        self.plugin.devices_mut().remove(&device);
        if self.house.registry.remove(&device).is_some() {
            self.publish(Event::DeviceRemoved { id: device });
        }
        let store = self.house.store.clone();
        let plugin = self.id().to_string();
        if let Err(e) = self.wait(async { store.remove_device(&plugin, &id).await }) {
            tracing::warn!(plugin = %plugin, "could not forget {id}: {e}");
        }
        Ok(())
    }

    fn registered(&mut self) -> Vec<String> {
        if self.may_use_devices().is_err() {
            return Vec::new();
        }
        let mut ids: Vec<String> = self
            .plugin
            .devices()
            .values()
            .map(|d| d.id.clone())
            .collect();
        ids.sort();
        ids
    }
}

impl timers::Host for HostState {
    fn every(&mut self, id: String, seconds: u32) -> Result<(), HostError> {
        if seconds < 5 {
            return Err(HostError::Invalid("every is at least 5 seconds".into()));
        }
        self.set_timer(id, seconds, true)
    }

    fn after(&mut self, id: String, seconds: u32) -> Result<(), HostError> {
        if seconds < 1 {
            return Err(HostError::Invalid("after is at least 1 second".into()));
        }
        self.set_timer(id, seconds, false)
    }

    fn cancel(&mut self, id: String) {
        self.timers.remove(&id);
    }
}

impl HostState {
    fn set_timer(&mut self, id: String, seconds: u32, repeat: bool) -> Result<(), HostError> {
        if !self.timers.contains_key(&id) && self.timers.len() >= MAX_TIMERS {
            return Err(HostError::Limit(format!("at most {MAX_TIMERS} timers")));
        }
        let period = Duration::from_secs(u64::from(seconds));
        self.timers.insert(
            id,
            Timer {
                every: repeat.then_some(period),
                due: Instant::now() + period,
            },
        );
        Ok(())
    }
}

pub(crate) fn event_permission(kind: types::EventKind) -> EventPermission {
    match kind {
        types::EventKind::DeviceState => EventPermission::DeviceState,
        types::EventKind::DeviceAction => EventPermission::DeviceAction,
        types::EventKind::Presence => EventPermission::Presence,
    }
}

/// `room/name`, with `*` for either part.
fn check_pattern(pattern: &str) -> bool {
    let part = |p: &str| p == "*" || crate::manifest::is_identifier(p);
    matches!(pattern.split_once('/'), Some((room, name)) if part(room) && part(name))
}

impl events::Host for HostState {
    fn subscribe(&mut self, kind: types::EventKind, devices: Vec<String>) -> Result<(), HostError> {
        let kind = event_permission(kind);
        if !self.plugin.manifest.permissions.events.contains(&kind) {
            return Err(HostError::NotPermitted(format!(
                "the manifest's events permission does not list {kind:?}"
            )));
        }
        if devices.len() > MAX_PATTERNS {
            return Err(HostError::Limit(format!("at most {MAX_PATTERNS} patterns")));
        }
        if let Some(bad) = devices.iter().find(|p| !check_pattern(p)) {
            return Err(HostError::Invalid(format!(
                "{bad:?} is not a pattern like office/door, office/* or */door"
            )));
        }
        self.plugin.subscribe(kind, devices);
        Ok(())
    }

    fn unsubscribe(&mut self, kind: types::EventKind) {
        self.plugin.unsubscribe(event_permission(kind));
    }
}

impl notify::Host for HostState {
    fn say(
        &mut self,
        message: String,
        priority: notify::Priority,
        room: Option<String>,
    ) -> Result<(), HostError> {
        let granted = self.plugin.manifest.permissions.notify;
        match (granted, priority) {
            (Notify::No, _) => {
                return Err(HostError::NotPermitted(
                    "the manifest does not ask for notify".into(),
                ));
            }
            (Notify::Yes, notify::Priority::Urgent) => {
                return Err(HostError::NotPermitted(
                    "urgent needs notify = \"urgent\" in the manifest".into(),
                ));
            }
            _ => {}
        }
        let message = message.trim();
        if message.is_empty() || message.chars().count() > MAX_MESSAGE {
            return Err(HostError::Invalid(format!(
                "a message is 1 to {MAX_MESSAGE} characters"
            )));
        }
        if let Some(room) = &room
            && !crate::manifest::is_identifier(room)
        {
            return Err(HostError::Invalid(format!(
                "{room:?} is not a room id like living_room"
            )));
        }
        if !self.plugin.may_notify(NOTIFICATIONS_PER_HOUR) {
            return Err(HostError::Limit(format!(
                "at most {NOTIFICATIONS_PER_HOUR} notifications an hour"
            )));
        }
        let priority = match priority {
            notify::Priority::Low => niles_notifications::Priority::Routine,
            notify::Priority::Normal => niles_notifications::Priority::Important,
            notify::Priority::Urgent => niles_notifications::Priority::Urgent,
        };
        match self.house.notifications.get() {
            Some(center) => {
                // Delivery spawns the speaking onto the runtime; this
                // thread is not one of its workers.
                let _runtime = self.house.handle.enter();
                center.deliver(message, room, priority);
            }
            None => tracing::info!(
                plugin = %self.plugin.id,
                "would say {message:?} ({priority:?}) in {}",
                room.as_deref().unwrap_or("the room Niles picks")
            ),
        }
        Ok(())
    }
}

impl ui::Host for HostState {
    fn invalidate(&mut self, surface_id: String) {
        let manifest = &self.plugin.manifest;
        if !manifest.permissions.ui || !manifest.declares_surface(&surface_id) {
            tracing::debug!(
                plugin = %self.plugin.id,
                "ui.invalidate({surface_id:?}): no such surface"
            );
            return;
        }
        self.plugin.invalidate(&surface_id, &self.house);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> types::DeviceState {
        types::DeviceState {
            on: None,
            brightness: None,
            color_temp_kelvin: None,
            rgb: None,
            temperature_celsius: None,
            humidity_percent: None,
            battery_percent: None,
            open: None,
            motion: None,
            illuminance_lux: None,
            position: None,
            readings: Vec::new(),
        }
    }

    fn reading(name: &str, value: f64) -> types::Reading {
        types::Reading {
            name: name.into(),
            value,
            unit: Some("W".into()),
        }
    }

    #[test]
    fn readings_arrive_by_name_with_their_units() {
        let mut s = state();
        s.readings = vec![reading("power", 340.0)];
        let core = state_from_wit(s).unwrap();
        assert_eq!(core.readings["power"].value, 340.0);
        assert_eq!(core.readings["power"].unit.as_deref(), Some("W"));
        let back = state_to_wit(&core);
        assert_eq!(back.readings[0].name, "power");
    }

    #[test]
    fn a_fixed_field_is_not_a_reading() {
        let mut s = state();
        s.readings = vec![reading("temperature", 21.0)];
        assert!(matches!(state_from_wit(s), Err(HostError::Invalid(_))));
    }

    #[test]
    fn readings_names_and_values_are_checked() {
        let mut s = state();
        s.readings = vec![reading("Power", 1.0)];
        assert!(state_from_wit(s).is_err());
        let mut s = state();
        s.readings = vec![reading("power", f64::NAN)];
        assert!(state_from_wit(s).is_err());
        let mut s = state();
        s.readings = (0..33).map(|i| reading(&format!("r{i}"), 1.0)).collect();
        assert!(matches!(state_from_wit(s), Err(HostError::Limit(_))));
    }

    #[test]
    fn percentages_stay_in_range() {
        let mut s = state();
        s.brightness = Some(101);
        assert!(state_from_wit(s).is_err());
        let mut s = state();
        s.position = Some(100);
        assert!(state_from_wit(s).is_ok());
    }

    #[test]
    fn event_patterns_are_room_and_name() {
        assert!(check_pattern("office/door"));
        assert!(check_pattern("office/*"));
        assert!(check_pattern("*/door"));
        assert!(!check_pattern("office"));
        assert!(!check_pattern("Office/door"));
        assert!(!check_pattern("z2m:office/door"));
    }

    #[test]
    fn the_memory_cap_counts_every_memory() {
        use wasmtime::ResourceLimiter;
        let mut cap = MemoryCap::new(1);
        assert!(cap.memory_growing(0, 600 * 1024, None).unwrap());
        assert!(cap.memory_growing(0, 300 * 1024, None).unwrap());
        assert!(
            cap.memory_growing(0, 300 * 1024, None).is_err(),
            "a third memory past 1 MiB"
        );
        assert!(cap.memory_growing(300 * 1024, 400 * 1024, None).unwrap());
    }
}
