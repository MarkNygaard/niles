//! `niles-integration.toml`: what a plugin is, what it may reach, and
//! what its setup panel asks for.
//!
//! The rules are those of `site/content/docs/writing-add-ons/manifest.mdx`. Parsing is
//! strict about the shape — an unknown key is an error, because a typo
//! in `[permissions]` would otherwise quietly take a permission away —
//! and [`Manifest::problems`] then lists everything else wrong, all at
//! once, so `niles plugin check` can say it in one go.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The Niles this is, for `min_niles`.
pub const NILES_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `interface` versions this Niles runs, oldest first.
pub const INTERFACES: &[&str] = &["0.1", "0.2", "0.3", "0.4", "0.5", "0.6"];

/// Memory a plugin gets unless its manifest asks for more.
pub const DEFAULT_MEMORY_MB: u32 = 64;
/// The most a manifest may ask for.
pub const MAX_MEMORY_MB: u32 = 256;

/// At most this many tools per plugin.
pub const MAX_TOOLS: usize = 16;

/// Readings names that belong to a fixed field of the device state.
pub const TAKEN_READINGS: &[&str] = &[
    "temperature",
    "humidity",
    "battery",
    "brightness",
    "illuminance",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    pub min_niles: String,
    pub interface: String,
    /// A catalogue group id. Missing or unknown means Custom, which is
    /// why it is not checked.
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub memory_mb: Option<u32>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub settings: Vec<Setting>,
    #[serde(default)]
    pub ui: Surfaces,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Host names, exact or `*.example.com`.
    #[serde(default)]
    pub http: Vec<String>,
    /// IPs, host names, CIDR ranges no wider than a /24, or
    /// `setting:<key>`.
    #[serde(default)]
    pub local_network: Vec<String>,
    #[serde(default)]
    pub devices: bool,
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub events: Vec<EventPermission>,
    #[serde(default)]
    pub notify: Notify,
    #[serde(default)]
    pub ui: bool,
    /// MQTT brokers, from interface 0.2.
    #[serde(default)]
    pub mqtt: Vec<MqttBroker>,
    /// Connect an account through its sign-in page, from interface 0.3.
    #[serde(default)]
    pub callback: bool,
    /// A secret address the internet may call, from interface 0.3.
    #[serde(default)]
    pub webhook: bool,
    /// Named WebSocket endpoints. Introduced in interface 0.4.
    #[serde(default)]
    pub websocket: Vec<WebSocketGrant>,
    /// Permission to use Niles's room-bound text speech stream (interface 0.5).
    #[serde(default)]
    pub speech_stream: Option<SpeechStreamGrant>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechStreamGrant {
    /// The declared room setting whose current value fixes the session room.
    pub room: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSocketGrant {
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub tls: bool,
}

/// One broker a plugin may connect to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttBroker {
    /// What `mqtt.connect` calls it: `[a-z0-9_]+`, unique.
    pub name: String,
    /// A host name, an IP address, or `setting:<key>` for the address
    /// the household types in.
    pub host: String,
    /// 8883, or 1883 with `tls = "none"`, unless given.
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub tls: TlsMode,
}

/// How a broker's certificate is checked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
pub enum TlsMode {
    /// Against the public certificate authorities, for the host's name.
    #[default]
    #[serde(rename = "verify")]
    Verify,
    /// The device's own certificate, self-signed or not, trusted the
    /// first time and pinned from then on. Home network only.
    #[serde(rename = "trust-device")]
    TrustDevice,
    /// No TLS at all. Home network only.
    #[serde(rename = "none")]
    Plain,
}

impl TlsMode {
    /// As the manifest writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            TlsMode::Verify => "verify",
            TlsMode::TrustDevice => "trust-device",
            TlsMode::Plain => "none",
        }
    }
}

impl MqttBroker {
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(self.default_port())
    }

    /// MQTT's own: 8883 over TLS, 1883 without.
    pub fn default_port(&self) -> u16 {
        match self.tls {
            TlsMode::Plain => 1883,
            _ => 8883,
        }
    }

    /// The setting the address comes from, for `setting:<key>`.
    pub fn setting(&self) -> Option<&str> {
        self.host.strip_prefix("setting:")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPermission {
    DeviceState,
    DeviceAction,
    Presence,
}

/// `notify = true`, or `"urgent"` to speak during quiet hours too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Notify {
    #[default]
    No,
    Yes,
    Urgent,
}

impl<'de> Deserialize<'de> for Notify {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flag(bool),
            Word(String),
        }
        match Raw::deserialize(d)? {
            Raw::Flag(false) => Ok(Notify::No),
            Raw::Flag(true) => Ok(Notify::Yes),
            Raw::Word(w) if w == "urgent" => Ok(Notify::Urgent),
            Raw::Word(w) => Err(serde::de::Error::custom(format!(
                "notify is true, false or \"urgent\", not {w:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: SettingKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub default: Option<toml::Value>,
    #[serde(default)]
    pub options: Vec<ChoiceOption>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    String,
    Number,
    Boolean,
    Secret,
    Choice,
    Room,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surfaces {
    #[serde(default)]
    pub menu: Vec<MenuEntry>,
    #[serde(default)]
    pub widgets: Vec<Widget>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MenuEntry {
    pub id: String,
    pub label: String,
    pub icon: String,
    #[serde(default)]
    pub refresh_every: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Widget {
    pub id: String,
    pub title: String,
    /// The room it starts in: a room's name, or `setting:<key>` for the
    /// room a `room` setting holds. None, or that setting unset, is the
    /// front page.
    #[serde(default)]
    pub room: Option<String>,
    #[serde(default)]
    pub refresh_every: Option<u32>,
}

impl Widget {
    /// The `room` setting that places it, when `room` is `setting:<key>`.
    pub fn room_setting(&self) -> Option<&str> {
        self.room.as_deref()?.strip_prefix("setting:")
    }
}

/// What a setting holds, as the plugin is given it.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Text(String),
    Number(f64),
    Boolean(bool),
}

impl Manifest {
    /// Parse `niles-integration.toml`. Only the shape is checked here;
    /// see [`problems`](Self::problems) for the rest.
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| Error::Manifest(format!("does not parse: {e}")))
    }

    /// Parse, and refuse anything [`problems`](Self::problems) finds.
    pub fn parse_valid(text: &str) -> Result<Self> {
        let manifest = Self::parse(text)?;
        let problems = manifest.problems();
        if problems.is_empty() {
            Ok(manifest)
        } else {
            Err(Error::Manifest(format!(
                "is not valid: {}",
                problems.join("; ")
            )))
        }
    }

    /// Everything wrong with this manifest, each as a sentence for its
    /// author. Empty when there is nothing.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.check_identity(&mut out);
        self.check_permissions(&mut out);
        self.check_settings(&mut out);
        self.check_surfaces(&mut out);
        out
    }

    /// The device source this plugin's devices are kept under.
    pub fn source(&self) -> String {
        format!("x_{}", self.id)
    }

    pub fn memory_mb(&self) -> u32 {
        self.memory_mb.unwrap_or(DEFAULT_MEMORY_MB)
    }

    pub fn setting(&self, key: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.key == key)
    }

    /// The room `widget` starts in with these settings: its `room`, or
    /// the one its `setting:` names. None is the front page, which is
    /// also where it is while that setting is empty.
    pub fn widget_room(&self, widget: &Widget, values: &toml::Table) -> Option<String> {
        let Some(key) = widget.room_setting() else {
            return widget.room.clone();
        };
        match self.setting(key)?.value(values)? {
            SettingValue::Text(room) if !room.is_empty() => Some(room),
            _ => None,
        }
    }

    /// Whether `id` is one of this plugin's declared pages or widgets.
    pub fn declares_surface(&self, id: &str) -> bool {
        self.ui.menu.iter().any(|m| m.id == id) || self.ui.widgets.iter().any(|w| w.id == id)
    }

    /// How long a render of `surface` may be served from the cache:
    /// its `refresh_every`, if it has one.
    pub fn refresh_every(&self, surface: &str) -> Option<u32> {
        self.ui
            .menu
            .iter()
            .find(|m| m.id == surface)
            .and_then(|m| m.refresh_every)
            .or_else(|| {
                self.ui
                    .widgets
                    .iter()
                    .find(|w| w.id == surface)
                    .and_then(|w| w.refresh_every)
            })
    }

    /// The labels of required settings that have no value: neither one
    /// the household entered nor a default. Until this is empty the
    /// plugin is "Not set up" and does not start.
    pub fn missing_required(
        &self,
        values: &toml::Table,
        secret_set: impl Fn(&str) -> bool,
    ) -> Vec<&str> {
        self.settings
            .iter()
            .filter(|s| s.required)
            .filter(|s| match s.kind {
                SettingKind::Secret => !secret_set(&s.key),
                _ => s.value(values).is_none(),
            })
            .map(|s| s.label.as_str())
            .collect()
    }

    fn check_identity(&self, out: &mut Vec<String>) {
        if let Err(why) = check_id(&self.id) {
            out.push(why);
        }
        if self.name.trim().is_empty() {
            out.push("name is empty".into());
        }
        if semver::Version::parse(&self.version).is_err() {
            out.push(format!(
                "version {:?} is not a semver version such as 0.1.0",
                self.version
            ));
        }
        let description = self.description.trim();
        if description.is_empty() {
            out.push("description is empty".into());
        } else if description.chars().count() > 120 {
            out.push("description is longer than 120 characters".into());
        } else if description.contains('\n') {
            out.push("description is more than one line".into());
        }
        if self.authors.is_empty() || self.authors.iter().any(|a| a.trim().is_empty()) {
            out.push("authors must name at least one person, and none may be empty".into());
        }
        match semver::Version::parse(&self.min_niles) {
            Err(_) => out.push(format!(
                "min_niles {:?} is not a semver version such as 0.2.0",
                self.min_niles
            )),
            Ok(min) => {
                let this = semver::Version::parse(NILES_VERSION).expect("our own version");
                if min > this {
                    out.push(format!(
                        "it needs Niles {min} or newer, and this is Niles {this}"
                    ));
                }
            }
        }
        if !INTERFACES.contains(&self.interface.as_str()) {
            out.push(format!(
                "interface {:?} is not one this Niles runs ({})",
                self.interface,
                INTERFACES.join(", ")
            ));
        }
        if let Some(logo) = &self.logo
            && !logo.to_ascii_lowercase().ends_with(".svg")
        {
            out.push(format!("logo {logo:?} must be an SVG"));
        }
        if let Some(mb) = self.memory_mb
            && !(1..=MAX_MEMORY_MB).contains(&mb)
        {
            out.push(format!(
                "memory_mb is {mb}; it must be between 1 and {MAX_MEMORY_MB}"
            ));
        }
        if self.keywords.iter().any(|k| k.trim().is_empty()) {
            out.push("keywords may not be empty strings".into());
        }
    }

    fn check_permissions(&self, out: &mut Vec<String>) {
        let p = &self.permissions;
        for host in &p.http {
            if let Err(why) = crate::policy::check_host_pattern(host) {
                out.push(format!("http: {why}"));
            }
        }
        for entry in &p.local_network {
            match entry.strip_prefix("setting:") {
                Some(key) => match self.setting(key) {
                    Some(s) if matches!(s.kind, SettingKind::String) => {}
                    Some(_) => out.push(format!(
                        "local_network: setting:{key} must name a string setting"
                    )),
                    None => out.push(format!(
                        "local_network: setting:{key} names no declared setting"
                    )),
                },
                None => {
                    if let Err(why) = crate::policy::check_local_entry(entry) {
                        out.push(format!("local_network: {why}"));
                    }
                }
            }
        }
        if p.tools && self.keywords.is_empty() {
            out.push(
                "a plugin with tools needs keywords: the words that make its tools \
                 relevant to a request"
                    .into(),
            );
        }
        let mut events = HashSet::new();
        for kind in &p.events {
            if !events.insert(kind) {
                out.push(format!("events lists {kind:?} twice"));
            }
        }
        let has_surfaces = !self.ui.menu.is_empty() || !self.ui.widgets.is_empty();
        if has_surfaces && !p.ui {
            out.push("pages and widgets need ui = true in [permissions]".into());
        }
        self.check_mqtt(out);
        self.check_websocket(out);
        let older = matches!(self.interface.as_str(), "0.1" | "0.2");
        for (asked, name) in [(p.callback, "callback"), (p.webhook, "webhook")] {
            if asked && older {
                out.push(format!(
                    "{name}: accounts and webhooks are in interface 0.3; set interface = \"0.3\" \
                     and build against its WIT"
                ));
            }
        }
        if p.callback && p.http.is_empty() {
            out.push(
                "callback: the sign-in page has to be on a host the http permission declares"
                    .into(),
            );
        }
    }

    fn check_mqtt(&self, out: &mut Vec<String>) {
        let brokers = &self.permissions.mqtt;
        if !brokers.is_empty() && self.interface == "0.1" {
            out.push(
                "mqtt: MQTT is in interface 0.2; set interface = \"0.2\" and build against its WIT"
                    .into(),
            );
        }
        let mut names = HashSet::new();
        for broker in brokers {
            let name = &broker.name;
            if !is_identifier(name) || name.len() > 32 {
                out.push(format!(
                    "mqtt: the broker name {name:?} must be [a-z0-9_]+, at most 32 characters"
                ));
            }
            if !names.insert(name.as_str()) {
                out.push(format!("mqtt: the broker name {name:?} is used twice"));
            }
            if broker.port == Some(0) {
                out.push(format!("mqtt: broker {name:?} has port 0"));
            }
            match broker.setting() {
                Some(key) => match self.setting(key) {
                    Some(s) if matches!(s.kind, SettingKind::String) => {}
                    Some(_) => out.push(format!(
                        "mqtt: broker {name:?}: setting:{key} must name a string setting"
                    )),
                    None => out.push(format!(
                        "mqtt: broker {name:?}: setting:{key} names no declared setting"
                    )),
                },
                None => {
                    if let Err(why) = crate::policy::check_mqtt_host(&broker.host, broker.tls) {
                        out.push(format!("mqtt: broker {name:?}: {why}"));
                    }
                }
            }
        }
    }

    fn check_websocket(&self, out: &mut Vec<String>) {
        let mut names = HashSet::new();
        for grant in &self.permissions.websocket {
            if !is_identifier(&grant.name) || grant.name.len() > 32 {
                out.push(format!(
                    "websocket: grant name {:?} must be [a-z0-9_]+, at most 32 characters",
                    grant.name
                ));
            }
            if !names.insert(grant.name.as_str()) {
                out.push(format!(
                    "websocket: grant name {:?} is used twice",
                    grant.name
                ));
            }
            if grant.port == Some(0) {
                out.push(format!("websocket: grant {:?} has port 0", grant.name));
            }
            match grant.host.strip_prefix("setting:") {
                Some(key) => match self.setting(key) {
                    Some(s) if s.kind == SettingKind::String => {}
                    Some(_) => out.push(format!(
                        "websocket: setting:{key} must name a string setting"
                    )),
                    None => out.push(format!(
                        "websocket: setting:{key} names no declared setting"
                    )),
                },
                None => {
                    if let Err(why) = crate::policy::check_mqtt_host(
                        &grant.host,
                        if grant.tls {
                            TlsMode::Verify
                        } else {
                            TlsMode::Plain
                        },
                    ) {
                        out.push(format!("websocket: {why}"));
                    }
                    if !grant.tls {
                        if let Ok(ip) = grant
                            .host
                            .trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                        {
                            if !crate::policy::is_local_network_address(ip) {
                                out.push(
                                    "websocket: plaintext grants must be on local_network".into(),
                                );
                            }
                        } else if !self.permissions.local_network.iter().any(|entry| {
                            entry
                                .trim_end_matches('.')
                                .eq_ignore_ascii_case(grant.host.trim_end_matches('.'))
                        }) {
                            out.push(
                                "websocket: plaintext host must be declared in local_network"
                                    .into(),
                            );
                        }
                    }
                }
            }
        }
        if !self.permissions.websocket.is_empty()
            && self.interface != "0.4"
            && self.interface != "0.5"
            && self.interface != "0.6"
        {
            out.push("websocket: WebSockets are in interface 0.4; set interface = \"0.4\" and build against its WIT".into());
        }
        if self.permissions.speech_stream.is_some()
            && self.interface != "0.5"
            && self.interface != "0.6"
        {
            out.push("speech_stream: the speech stream is in interface 0.5; set interface = \"0.5\" and build against its WIT".into());
        }
        if let Some(grant) = &self.permissions.speech_stream {
            match self.setting(&grant.room) {
                Some(setting) if setting.kind == SettingKind::Room => {}
                Some(_) => out.push(format!(
                    "speech_stream: room setting {:?} must have type room",
                    grant.room
                )),
                None => out.push(format!(
                    "speech_stream: room setting {:?} is not declared",
                    grant.room
                )),
            }
            match self.setting("speech_stream_enabled") {
                Some(setting) if setting.kind == SettingKind::Boolean
                    && setting.default.as_ref().is_none_or(|v| matches!(v, toml::Value::Boolean(false))) => {}
                Some(_) => out.push("speech_stream: speech_stream_enabled must be a boolean setting with no default or default = false".into()),
                None => out.push("speech_stream: requires a boolean speech_stream_enabled setting with no default or default = false".into()),
            }
        }
    }

    fn check_settings(&self, out: &mut Vec<String>) {
        let mut keys = HashSet::new();
        for s in &self.settings {
            if !is_identifier(&s.key) {
                out.push(format!("setting key {:?} must be [a-z0-9_]+", s.key));
            }
            if !keys.insert(s.key.as_str()) {
                out.push(format!("setting key {:?} is declared twice", s.key));
            }
            if s.label.trim().is_empty() {
                out.push(format!("setting {:?} has no label", s.key));
            }
            if s.kind == SettingKind::Choice && s.options.is_empty() {
                out.push(format!("choice setting {:?} has no options", s.key));
            }
            if s.kind != SettingKind::Choice && !s.options.is_empty() {
                out.push(format!(
                    "setting {:?} has options but is not a choice",
                    s.key
                ));
            }
            if (s.min.is_some() || s.max.is_some()) && s.kind != SettingKind::Number {
                out.push(format!(
                    "setting {:?} has min or max but is not a number",
                    s.key
                ));
            }
            if let (Some(min), Some(max)) = (s.min, s.max)
                && min > max
            {
                out.push(format!("setting {:?} has min above max", s.key));
            }
            if let Some(default) = &s.default {
                if s.kind == SettingKind::Secret {
                    out.push(format!("secret setting {:?} cannot have a default", s.key));
                } else if let Err(why) = s.accepts(default) {
                    out.push(format!("setting {:?}: the default {why}", s.key));
                }
            }
        }
    }

    fn check_surfaces(&self, out: &mut Vec<String>) {
        let mut ids = HashSet::new();
        let declared = self
            .ui
            .menu
            .iter()
            .map(|m| (&m.id, m.refresh_every))
            .chain(self.ui.widgets.iter().map(|w| (&w.id, w.refresh_every)));
        for (id, refresh) in declared {
            if !is_identifier(id) {
                out.push(format!("surface id {id:?} must be [a-z0-9_]+"));
            }
            if !ids.insert(id.as_str()) {
                out.push(format!("surface id {id:?} is declared twice"));
            }
            if refresh.is_some_and(|s| s < 10) {
                out.push(format!(
                    "surface {id:?}: refresh_every is at least 10 seconds"
                ));
            }
        }
        for m in &self.ui.menu {
            if m.label.trim().is_empty() {
                out.push(format!("menu entry {:?} has no label", m.id));
            }
            if !is_icon_name(&m.icon) {
                out.push(format!(
                    "menu entry {:?}: {:?} is not a Lucide icon name (lowercase words joined by -)",
                    m.id, m.icon
                ));
            }
        }
        for w in &self.ui.widgets {
            if w.title.trim().is_empty() {
                out.push(format!("widget {:?} has no title", w.id));
            }
            if let Some(key) = w.room_setting() {
                if self
                    .setting(key)
                    .is_none_or(|s| s.kind != SettingKind::Room)
                {
                    out.push(format!(
                        "widget {:?}: room \"setting:{key}\" must name a setting of type room",
                        w.id
                    ));
                }
            } else if let Some(room) = &w.room
                && !is_identifier(room)
            {
                out.push(format!(
                    "widget {:?}: room {room:?} must be [a-z0-9_]+, or setting:<key> for a room setting",
                    w.id
                ));
            }
        }
    }
}

impl Setting {
    /// The value the plugin sees: what is in `values`, else the default.
    /// Never a secret's: those come through `config.secret` only.
    pub fn value(&self, values: &toml::Table) -> Option<SettingValue> {
        if self.kind == SettingKind::Secret {
            return None;
        }
        values
            .get(&self.key)
            .filter(|v| self.accepts(v).is_ok())
            .or(self.default.as_ref())
            .and_then(|v| self.convert(v))
    }

    fn convert(&self, value: &toml::Value) -> Option<SettingValue> {
        match (self.kind, value) {
            (SettingKind::Number, toml::Value::Integer(n)) => Some(SettingValue::Number(*n as f64)),
            (SettingKind::Number, toml::Value::Float(n)) => Some(SettingValue::Number(*n)),
            (SettingKind::Boolean, toml::Value::Boolean(b)) => Some(SettingValue::Boolean(*b)),
            (
                SettingKind::String | SettingKind::Choice | SettingKind::Room,
                toml::Value::String(s),
            ) => Some(SettingValue::Text(s.clone())),
            _ => None,
        }
    }

    /// Whether `value` fits this setting; the error completes "the value …".
    pub fn accepts(&self, value: &toml::Value) -> std::result::Result<(), String> {
        match (self.kind, value) {
            (SettingKind::Number, toml::Value::Integer(_) | toml::Value::Float(_)) => {
                let n = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64))
                    .unwrap_or_default();
                if self.min.is_some_and(|min| n < min) || self.max.is_some_and(|max| n > max) {
                    return Err(format!("{n} is outside min and max"));
                }
                Ok(())
            }
            (SettingKind::Boolean, toml::Value::Boolean(_)) => Ok(()),
            (SettingKind::String | SettingKind::Secret, toml::Value::String(_)) => Ok(()),
            (SettingKind::Room, toml::Value::String(room)) if is_identifier(room) => Ok(()),
            (SettingKind::Room, toml::Value::String(room)) => {
                Err(format!("{room:?} is not a room id like living_room"))
            }
            (SettingKind::Choice, toml::Value::String(choice)) => {
                if self.options.iter().any(|o| &o.value == choice) {
                    Ok(())
                } else {
                    Err(format!("{choice:?} is not one of the options"))
                }
            }
            (kind, other) => Err(format!(
                "{other} is not a {}",
                match kind {
                    SettingKind::Number => "number",
                    SettingKind::Boolean => "boolean",
                    _ => "string",
                }
            )),
        }
    }

    /// A value typed on a command line, as this setting would hold it.
    pub fn parse_text(&self, text: &str) -> std::result::Result<toml::Value, String> {
        let value = match self.kind {
            SettingKind::Number => text
                .parse::<f64>()
                .map(toml::Value::Float)
                .map_err(|_| format!("{text:?} is not a number"))?,
            SettingKind::Boolean => text
                .parse::<bool>()
                .map(toml::Value::Boolean)
                .map_err(|_| format!("{text:?} is not true or false"))?,
            _ => toml::Value::String(text.to_string()),
        };
        self.accepts(&value)?;
        Ok(value)
    }
}

/// The rules for a plugin id, as a sentence when it breaks one.
pub fn check_id(id: &str) -> std::result::Result<(), String> {
    let length = id.chars().count();
    if !(2..=32).contains(&length) {
        return Err(format!("id {id:?} must be 2 to 32 characters"));
    }
    if !id.starts_with(|c: char| c.is_ascii_lowercase()) || !is_identifier(id) {
        return Err(format!(
            "id {id:?} must be lowercase letters, digits and _, starting with a letter"
        ));
    }
    if niles_config::catalogue::find(id).is_some() {
        return Err(format!("id {id:?} is the id of one of Niles's own add-ons"));
    }
    Ok(())
}

/// `[a-z0-9_]+`, the rule for room, device, setting, tool and surface names.
pub fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Lucide's names are lowercase words joined by hyphens: `zap`,
/// `battery-charging`. The list itself is the app's to know.
fn is_icon_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPOTPRICE: &str = r#"
id = "spotprice"
name = "Spot prices"
version = "0.1.0"
description = "Hear what power costs hour by hour, and let rules run things when it is cheapest."
authors = ["Jane Doe <jane@example.com>"]
homepage = "https://github.com/jane/niles-spotprice"
min_niles = "0.2.0"
interface = "0.1"
logo = "logo.svg"
keywords = ["electricity", "power", "strøm"]

[permissions]
http = ["api.energidataservice.dk"]
devices = true
tools = true
ui = true

[[settings]]
key = "area"
label = "Price area"
type = "choice"
required = true
default = "DK1"
options = [
    { value = "DK1", label = "West Denmark (DK1)" },
    { value = "DK2", label = "East Denmark (DK2)" },
]

[[settings]]
key = "cheap_hours"
label = "Cheap hours a day"
type = "number"
default = 4
min = 1
max = 12

[[ui.menu]]
id = "electricity"
label = "Electricity"
icon = "zap"
refresh_every = 300
"#;

    fn spotprice() -> Manifest {
        Manifest::parse(SPOTPRICE).expect("parses")
    }

    fn with(edit: &str) -> Vec<String> {
        Manifest::parse(&format!("{SPOTPRICE}\n{edit}"))
            .map(|m| m.problems())
            .unwrap_or_else(|e| vec![e.to_string()])
    }

    fn replacing(from: &str, to: &str) -> Vec<String> {
        assert!(SPOTPRICE.contains(from), "{from}");
        Manifest::parse(&SPOTPRICE.replace(from, to))
            .map(|m| m.problems())
            .unwrap_or_else(|e| vec![e.to_string()])
    }

    const ROOM_WIDGET: &str = r#"
[[settings]]
key = "room"
label = "Room"
type = "room"

[[ui.widgets]]
id = "now"
title = "Now"
room = "setting:room"
"#;

    #[test]
    fn a_widget_can_start_in_the_room_a_setting_names() {
        assert_eq!(with(ROOM_WIDGET), Vec::<String>::new());
        let manifest = Manifest::parse(&format!("{SPOTPRICE}\n{ROOM_WIDGET}")).unwrap();
        let widget = &manifest.ui.widgets[0];
        let mut values = toml::Table::new();
        assert_eq!(
            manifest.widget_room(widget, &values),
            None,
            "the front page"
        );
        values.insert("room".into(), "office".into());
        assert_eq!(
            manifest.widget_room(widget, &values).as_deref(),
            Some("office")
        );
        values.insert("room".into(), "".into());
        assert_eq!(manifest.widget_room(widget, &values), None);

        let fixed = Widget {
            room: Some("hallway".into()),
            ..widget.clone()
        };
        assert_eq!(
            manifest.widget_room(&fixed, &values).as_deref(),
            Some("hallway")
        );
    }

    #[test]
    fn a_widget_room_setting_must_be_a_declared_room_setting() {
        let undeclared = with(&ROOM_WIDGET.replace("setting:room", "setting:place"));
        assert!(
            undeclared.iter().any(|p| p.contains("setting:place")),
            "{undeclared:?}"
        );
        let not_a_room = with(&ROOM_WIDGET.replace("setting:room", "setting:area"));
        assert!(
            not_a_room.iter().any(|p| p.contains("of type room")),
            "{not_a_room:?}"
        );
        let bad = with(&ROOM_WIDGET.replace("setting:room", "Office!"));
        assert!(bad.iter().any(|p| p.contains("[a-z0-9_]+")), "{bad:?}");
    }

    #[test]
    fn the_worked_example_is_valid() {
        assert_eq!(spotprice().problems(), Vec::<String>::new());
        assert_eq!(spotprice().source(), "x_spotprice");
        assert_eq!(spotprice().memory_mb(), DEFAULT_MEMORY_MB);
    }

    #[test]
    fn speech_stream_requires_false_or_absent_boolean_and_room_settings() {
        let valid = format!(
            "{}\n[[settings]]\nkey = \"speech_stream_enabled\"\nlabel = \"Enabled\"\ntype = \"boolean\"\ndefault = false\n\n[[settings]]\nkey = \"room\"\nlabel = \"Room\"\ntype = \"room\"",
            SPOTPRICE
                .replace(
                    "devices = true",
                    "devices = true\nspeech_stream = { room = \"room\" }"
                )
                .replace("interface = \"0.1\"", "interface = \"0.5\"")
        );
        let manifest = Manifest::parse(&valid).unwrap();
        assert!(manifest.problems().is_empty(), "{:?}", manifest.problems());

        for (needle, replacement, expected) in [
            (
                "\nkey = \"speech_stream_enabled\"",
                "\nkey = \"other_enabled\"",
                "speech_stream_enabled",
            ),
            (
                "type = \"boolean\"",
                "type = \"string\"",
                "speech_stream_enabled",
            ),
            ("default = false", "default = true", "speech_stream_enabled"),
            ("type = \"room\"", "type = \"string\"", "room"),
            ("key = \"room\"", "key = \"other_room\"", "room setting"),
        ] {
            let invalid = valid.replacen(needle, replacement, 1);
            let parsed = Manifest::parse(&invalid).expect("invalid declarations still parse");
            assert!(
                parsed
                    .problems()
                    .iter()
                    .any(|problem| problem.contains(expected)),
                "{expected}: {:?}",
                parsed.problems()
            );
        }
    }

    #[test]
    fn an_unknown_key_is_an_error_not_a_lost_permission() {
        let problems = replacing("devices = true", "device = true");
        assert!(problems[0].contains("unknown field"), "{problems:?}");
    }

    #[test]
    fn ids_follow_the_rules() {
        assert!(check_id("airgradient").is_ok());
        assert!(check_id("a").is_err(), "too short");
        assert!(check_id("2fast").is_err(), "starts with a digit");
        assert!(check_id("Spot").is_err());
        assert!(check_id("spot-price").is_err());
        assert!(check_id(&"x".repeat(33)).is_err());
        assert!(check_id("sonos").is_err(), "a built-in's id");
    }

    #[test]
    fn a_newer_niles_or_another_interface_is_refused() {
        assert!(
            replacing(r#"min_niles = "0.2.0""#, r#"min_niles = "9.0.0""#)[0]
                .contains("needs Niles")
        );
        assert!(replacing(r#"interface = "0.1""#, r#"interface = "0.4""#).is_empty());
        assert!(replacing(r#"interface = "0.1""#, r#"interface = "0.2""#).is_empty());
        assert!(replacing(r#"interface = "0.1""#, r#"interface = "0.3""#).is_empty());
        assert!(replacing(r#"interface = "0.1""#, r#"interface = "0.6""#).is_empty());
    }

    #[test]
    fn accounts_and_webhooks_need_interface_0_3() {
        for permission in ["callback = true", "webhook = true"] {
            let edit = format!("devices = true\n{permission}");
            let on_0_1 = replacing("devices = true", &edit);
            assert!(
                on_0_1.iter().any(|p| p.contains("interface 0.3")),
                "{permission}: {on_0_1:?}"
            );
            let on_0_3 = Manifest::parse(
                &SPOTPRICE
                    .replace(r#"interface = "0.1""#, r#"interface = "0.3""#)
                    .replace("devices = true", &edit),
            )
            .unwrap();
            assert_eq!(on_0_3.problems(), Vec::<String>::new(), "{permission}");
        }
    }

    #[test]
    fn a_sign_in_page_needs_a_host_to_be_on() {
        let manifest = Manifest::parse(
            &SPOTPRICE
                .replace(r#"interface = "0.1""#, r#"interface = "0.3""#)
                .replace(r#"http = ["api.energidataservice.dk"]"#, "callback = true"),
        )
        .unwrap();
        assert!(
            manifest
                .problems()
                .iter()
                .any(|p| p.contains("callback") && p.contains("http")),
            "{:?}",
            manifest.problems()
        );
    }

    #[test]
    fn a_long_description_is_refused() {
        let long = format!(r#"description = "{}""#, "x".repeat(121));
        let problems = replacing(
            r#"description = "Hear what power costs hour by hour, and let rules run things when it is cheapest.""#,
            &long,
        );
        assert!(problems.iter().any(|p| p.contains("120")), "{problems:?}");
    }

    #[test]
    fn tools_need_keywords() {
        let problems = replacing(r#"keywords = ["electricity", "power", "strøm"]"#, "");
        assert!(
            problems.iter().any(|p| p.contains("keywords")),
            "{problems:?}"
        );
    }

    #[test]
    fn memory_is_capped() {
        let asking = |mb: u32| {
            replacing(
                r#"logo = "logo.svg""#,
                &format!("logo = \"logo.svg\"\nmemory_mb = {mb}"),
            )
        };
        assert!(asking(512).iter().any(|p| p.contains("memory_mb")));
        assert!(asking(128).is_empty());
    }

    #[test]
    fn notify_takes_a_flag_or_urgent() {
        let m =
            Manifest::parse(&SPOTPRICE.replace("ui = true\n", "ui = true\nnotify = \"urgent\"\n"))
                .unwrap();
        assert_eq!(m.permissions.notify, Notify::Urgent);
        assert!(
            Manifest::parse(&SPOTPRICE.replace("ui = true\n", "ui = true\nnotify = \"loud\"\n"))
                .is_err()
        );
    }

    #[test]
    fn settings_defaults_must_fit_their_type() {
        assert!(
            replacing("default = 4", r#"default = "four""#)
                .iter()
                .any(|p| p.contains("cheap_hours"))
        );
        assert!(
            replacing(r#"default = "DK1""#, r#"default = "DK3""#)
                .iter()
                .any(|p| p.contains("not one of the options"))
        );
        assert!(
            replacing("default = 4", "default = 40")
                .iter()
                .any(|p| p.contains("outside"))
        );
    }

    #[test]
    fn a_secret_has_no_default() {
        let problems = with(
            "[[settings]]\nkey = \"api_key\"\nlabel = \"API key\"\ntype = \"secret\"\ndefault = \"hunter2\"\n",
        );
        assert!(
            problems.iter().any(|p| p.contains("secret")),
            "{problems:?}"
        );
    }

    #[test]
    fn setting_keys_are_unique() {
        let problems = with("[[settings]]\nkey = \"area\"\nlabel = \"Again\"\ntype = \"string\"\n");
        assert!(problems.iter().any(|p| p.contains("twice")), "{problems:?}");
    }

    #[test]
    fn surfaces_need_the_ui_permission_and_good_ids() {
        assert!(
            replacing("ui = true\n", "")
                .iter()
                .any(|p| p.contains("ui = true"))
        );
        assert!(
            replacing(r#"id = "electricity""#, r#"id = "Electricity""#)
                .iter()
                .any(|p| p.contains("surface id"))
        );
        assert!(
            replacing(r#"icon = "zap""#, r#"icon = "Zap Icon""#)
                .iter()
                .any(|p| p.contains("Lucide"))
        );
        assert!(
            replacing("refresh_every = 300", "refresh_every = 5")
                .iter()
                .any(|p| p.contains("at least 10"))
        );
    }

    #[test]
    fn local_network_settings_must_be_declared_strings() {
        let problems = replacing(
            r#"http = ["api.energidataservice.dk"]"#,
            "http = [\"api.energidataservice.dk\"]\nlocal_network = [\"setting:address\", \"setting:area\"]",
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("setting:address names no")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("setting:area must name a string")),
            "{problems:?}"
        );
    }

    /// The worked example on interface 0.2, with `brokers` as its mqtt
    /// permission and an address setting.
    fn with_brokers(brokers: &str) -> std::result::Result<Manifest, Vec<String>> {
        let text = SPOTPRICE
            .replace(r#"interface = "0.1""#, r#"interface = "0.2""#)
            .replace("ui = true\n", &format!("ui = true\nmqtt = [{brokers}]\n"))
            + "\n[[settings]]\nkey = \"address\"\nlabel = \"Printer address\"\ntype = \"string\"\n";
        let manifest = Manifest::parse(&text).map_err(|e| vec![e.to_string()])?;
        let problems = manifest.problems();
        if problems.is_empty() {
            Ok(manifest)
        } else {
            Err(problems)
        }
    }

    #[test]
    fn a_broker_takes_mqtts_port_and_verifies_by_default() {
        let m = with_brokers(r#"{ name = "cloud", host = "broker.example.com" }"#).unwrap();
        let broker = &m.permissions.mqtt[0];
        assert_eq!(broker.tls, TlsMode::Verify);
        assert_eq!(broker.port(), 8883);
        let m = with_brokers(
            r#"{ name = "plugs", host = "192.168.1.20", tls = "none" },
               { name = "printer", host = "setting:address", port = 8883, tls = "trust-device" }"#,
        )
        .unwrap();
        assert_eq!(m.permissions.mqtt[0].port(), 1883);
        assert_eq!(m.permissions.mqtt[1].setting(), Some("address"));
    }

    #[test]
    fn mqtt_needs_interface_0_2() {
        let text = SPOTPRICE.replace(
            "ui = true\n",
            "ui = true\nmqtt = [{ name = \"b\", host = \"broker.example.com\" }]\n",
        );
        let problems = Manifest::parse(&text).unwrap().problems();
        assert!(
            problems.iter().any(|p| p.contains("interface = \"0.2\"")),
            "{problems:?}"
        );
    }

    #[test]
    fn trust_device_and_plain_mqtt_are_for_the_home_network_only() {
        let refused = |broker: &str| with_brokers(broker).unwrap_err().join("; ");
        assert!(
            refused(r#"{ name = "b", host = "8.8.8.8", tls = "trust-device" }"#)
                .contains("only for the home network")
        );
        assert!(
            refused(r#"{ name = "b", host = "8.8.8.8", tls = "none" }"#)
                .contains("only for the home network")
        );
        assert!(with_brokers(r#"{ name = "b", host = "8.8.8.8" }"#).is_ok());
        assert!(
            with_brokers(r#"{ name = "b", host = "192.168.1.40", tls = "trust-device" }"#).is_ok()
        );
        assert!(
            with_brokers(r#"{ name = "b", host = "printer.lan", tls = "trust-device" }"#).is_ok(),
            "a name is checked when it resolves"
        );
        assert!(refused(r#"{ name = "b", host = "127.0.0.1" }"#).contains("no plugin may reach"));
        assert!(
            refused(r#"{ name = "b", host = "localhost", tls = "none" }"#).contains("localhost")
        );
    }

    #[test]
    fn broker_entries_are_checked() {
        let refused = |broker: &str| with_brokers(broker).unwrap_err().join("; ");
        assert!(
            refused(r#"{ name = "b", host = "mqtts://broker.example.com" }"#).contains("no scheme")
        );
        assert!(refused(r#"{ name = "b", host = "broker.example.com:8883" }"#).contains("port"));
        assert!(refused(r#"{ name = "b", host = "*.example.com" }"#).contains("wildcards"));
        assert!(refused(r#"{ name = "B", host = "broker.example.com" }"#).contains("[a-z0-9_]+"));
        assert!(
            refused(
                r#"{ name = "b", host = "a.example.com" }, { name = "b", host = "b.example.com" }"#
            )
            .contains("twice")
        );
        assert!(refused(r#"{ name = "b", host = "setting:nope" }"#).contains("names no declared"));
        assert!(refused(r#"{ name = "b", host = "setting:area" }"#).contains("string setting"));
        assert!(refused(r#"{ name = "b", host = "x.example.com", port = 0 }"#).contains("port 0"));
        assert!(
            refused(r#"{ name = "b", host = "x.example.com", tls = "insecure" }"#)
                .contains("unknown variant"),
            "a misspelt mode is an error, not verify"
        );
    }

    #[test]
    fn a_default_answers_a_required_setting() {
        let m = spotprice();
        assert!(
            m.missing_required(&toml::Table::new(), |_| false)
                .is_empty()
        );
        let m = Manifest::parse(&SPOTPRICE.replace(r#"default = "DK1""#, "")).unwrap();
        assert_eq!(
            m.missing_required(&toml::Table::new(), |_| false),
            vec!["Price area"]
        );
        let mut entered = toml::Table::new();
        entered.insert("area".into(), "DK2".into());
        assert!(m.missing_required(&entered, |_| false).is_empty());
    }

    #[test]
    fn values_arrive_typed_and_a_bad_one_falls_back_to_the_default() {
        let m = spotprice();
        let mut values = toml::Table::new();
        values.insert("cheap_hours".into(), toml::Value::Integer(6));
        assert_eq!(
            m.setting("cheap_hours").unwrap().value(&values),
            Some(SettingValue::Number(6.0))
        );
        values.insert("area".into(), "nowhere".into());
        assert_eq!(
            m.setting("area").unwrap().value(&values),
            Some(SettingValue::Text("DK1".into()))
        );
    }

    #[test]
    fn command_line_values_are_typed_by_the_manifest() {
        let m = spotprice();
        assert_eq!(
            m.setting("cheap_hours").unwrap().parse_text("6"),
            Ok(toml::Value::Float(6.0))
        );
        assert!(m.setting("cheap_hours").unwrap().parse_text("six").is_err());
        assert!(m.setting("area").unwrap().parse_text("DK3").is_err());
    }
}
