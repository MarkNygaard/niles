//! What a plugin asks for, in the words the approval screen uses.
//!
//! The wording is `site/content/docs/writing-add-ons/permissions.mdx`'s, line for line.
//! Each line is a [`Grant`] with a key that names the permission itself
//! rather than how it is worded, so an update's difference is what it
//! may now do that it could not before — not a page label that changed,
//! or a tool that was added under a `tools` permission it already had.

use crate::manifest::{DEFAULT_MEMORY_MB, EventPermission, Manifest, Notify, TlsMode};
use serde::Serialize;
use std::collections::HashSet;

/// Always said last, whatever the plugin asks for.
pub const CANNOT: &str = "It cannot see your conversations, the people in your house, \
     other add-ons' passwords or Niles's database.";

/// One thing a plugin may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// What is granted, for comparing two versions: `http:api.example.com`,
    /// `devices`, `notify:urgent`.
    pub key: String,
    /// As the household reads it.
    pub text: String,
}

/// What an update adds and takes away, in plain words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Diff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// Every grant in `manifest`, in the order the approval screen lists
/// them. `tools` are the bare names its component offers, when they are
/// known: an update check reads only the manifest.
#[expect(
    clippy::too_many_lines,
    reason = "one block per permission kind, in the order the approval screen lists them"
)]
pub fn grants(manifest: &Manifest, tools: Option<&[String]>) -> Vec<Grant> {
    let p = &manifest.permissions;
    let mut out = Vec::new();
    let mut add = |key: String, text: String| out.push(Grant { key, text });

    for host in &p.http {
        let text = match host.strip_prefix("*.") {
            Some(domain) => format!("Connect to any address under {domain}"),
            None => format!("Connect to {host}"),
        };
        add(format!("http:{host}"), text);
    }
    for entry in &p.local_network {
        let text = match entry.strip_prefix("setting:") {
            Some(key) => format!(
                "Reach the device at the address you enter under {}, on your home network",
                manifest.setting(key).map_or(key, |s| s.label.as_str())
            ),
            None => {
                let addresses = entry
                    .split_once('/')
                    .and_then(|(_, bits)| bits.parse::<u32>().ok())
                    .and_then(|bits| 32u32.checked_sub(bits))
                    .map_or(1, |free| 1u64 << free.min(32));
                if addresses > 1 {
                    format!("Reach any of {addresses} addresses on your home network ({entry})")
                } else {
                    format!("Reach {entry} on your home network")
                }
            }
        };
        add(format!("local:{entry}"), text);
    }
    for broker in &p.mqtt {
        let mut text = match broker.setting() {
            Some(key) => format!(
                "Connect to the MQTT broker at the address you enter under {}",
                manifest.setting(key).map_or(key, |s| s.label.as_str())
            ),
            None => format!("Connect to the MQTT broker at {}", broker.host),
        };
        if broker.port() != broker.default_port() {
            text.push_str(&format!(", port {}", broker.port()));
        }
        text.push_str(match broker.tls {
            TlsMode::Verify => "",
            TlsMode::TrustDevice => {
                ", on your home network, trusting the certificate it shows the first time"
            }
            TlsMode::Plain => ", on your home network, unencrypted",
        });
        add(
            format!(
                "mqtt:{}:{}:{}:{}",
                broker.name,
                broker.host,
                broker.port(),
                broker.tls.as_str()
            ),
            text,
        );
    }
    for grant in &p.websocket {
        let location = match grant.host.strip_prefix("setting:") {
            Some(key) => format!(
                "the address you enter under {}",
                manifest.setting(key).map_or(key, |s| s.label.as_str())
            ),
            None => grant.host.clone(),
        };
        let port = grant.port.unwrap_or(if grant.tls { 443 } else { 80 });
        add(
            format!(
                "websocket:{}:{}:{}:{}",
                grant.name, grant.host, port, grant.tls
            ),
            format!(
                "Keep a connection open to {location}, port {port}, and receive what it sends{}",
                if grant.tls {
                    " over a TLS connection with certificate verification"
                } else {
                    " over an unencrypted connection on your home network"
                }
            ),
        );
    }
    if p.callback {
        add(
            "callback".into(),
            "Connect an account through its sign-in page, on a host it declares".into(),
        );
    }
    if p.webhook {
        add(
            "webhook".into(),
            "Receive messages from the internet at a secret address, if you make that address reachable"
                .into(),
        );
    }
    if p.devices {
        add(
            "devices".into(),
            "Add devices to your home. They appear in rooms, in rules and by voice.".into(),
        );
    }
    if p.tools {
        let text = match tools {
            Some([]) | None => "Offer tools to the assistant".to_string(),
            Some([one]) => format!("Offer 1 tool to the assistant: {one}"),
            Some(many) => format!(
                "Offer {} tools to the assistant: {}",
                many.len(),
                many.join(", ")
            ),
        };
        add("tools".into(), text);
    }
    for kind in &p.events {
        let (key, text) = match kind {
            EventPermission::DeviceState => (
                "device_state",
                "See every device in your home change: lights, doors, motion",
            ),
            EventPermission::DeviceAction => (
                "device_action",
                "See button presses and what other devices report",
            ),
            EventPermission::Presence => {
                ("presence", "Know when the house becomes occupied or empty")
            }
        };
        add(format!("events:{key}"), text.into());
    }
    match p.notify {
        Notify::No => {}
        Notify::Yes => add("notify".into(), "Speak through your satellites".into()),
        Notify::Urgent => add(
            "notify:urgent".into(),
            "Speak through your satellites, even during quiet hours".into(),
        ),
    }
    if p.speech_stream.is_some() {
        add(
            "speech_stream".into(),
            "Speak frequent commentary in a selected room when you explicitly enable it".into(),
        );
    }
    if p.ui {
        for entry in &manifest.ui.menu {
            add(
                format!("ui:menu:{}", entry.id),
                format!("Adds a page to the menu: {}", entry.label),
            );
        }
        for widget in &manifest.ui.widgets {
            let place = match (widget.room_setting(), &widget.room) {
                (Some(key), _) => {
                    let label = manifest.setting(key).map_or(key, |s| s.label.as_str());
                    format!("the room its \"{label}\" setting names")
                }
                (None, Some(room)) => format!("the {}", room_name(room)),
                (None, None) => "the front page".to_string(),
            };
            add(
                format!("ui:widget:{}", widget.id),
                format!("Adds a card to {place}: {}", widget.title),
            );
        }
    }
    let memory = manifest.memory_mb();
    if memory > DEFAULT_MEMORY_MB {
        add(
            format!("memory:{memory}"),
            format!("Use up to {memory} MB of memory"),
        );
    }
    out
}

/// `living_room` as a person says it: "Living room".
fn room_name(id: &str) -> String {
    let spoken = id.replace('_', " ");
    let mut chars = spoken.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => spoken,
    }
}

/// What `after` may do that `before` may not, and the reverse.
pub fn diff(before: &[Grant], after: &[Grant]) -> Diff {
    let keys =
        |grants: &[Grant]| -> HashSet<String> { grants.iter().map(|g| g.key.clone()).collect() };
    let (had, has) = (keys(before), keys(after));
    Diff {
        added: after
            .iter()
            .filter(|g| !had.contains(&g.key))
            .map(|g| g.text.clone())
            .collect(),
        removed: before
            .iter()
            .filter(|g| !has.contains(&g.key))
            .map(|g| g.text.clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
id = "spotprice"
name = "Spot prices"
version = "0.1.0"
description = "Prices."
authors = ["Jane"]
min_niles = "0.2.0"
interface = "0.1"
keywords = ["power"]

[permissions]
http = ["api.energidataservice.dk", "*.tibber.com"]
local_network = ["setting:address", "192.168.1.50", "192.168.20.0/24"]
devices = true
tools = true
events = ["device_state", "device_action", "presence"]
notify = "urgent"
ui = true

[[settings]]
key = "address"
label = "Address"
type = "string"

[[ui.menu]]
id = "electricity"
label = "Electricity"
icon = "zap"

[[ui.widgets]]
id = "now"
title = "Electricity now"

[[ui.widgets]]
id = "air"
title = "Air quality"
room = "living_room"
"#;

    fn texts(grants: &[Grant]) -> Vec<&str> {
        grants.iter().map(|g| g.text.as_str()).collect()
    }

    #[test]
    fn speech_stream_is_a_distinct_approval_grant() {
        let before = Manifest::parse(BASE).unwrap();
        let mut after = before.clone();
        after.permissions.speech_stream = Some(crate::manifest::SpeechStreamGrant {
            room: "room".into(),
        });
        let change = diff(&grants(&before, None), &grants(&after, None));
        assert_eq!(
            change.added,
            vec!["Speak frequent commentary in a selected room when you explicitly enable it"]
        );
        assert!(change.removed.is_empty());
        assert!(
            grants(&after, None)
                .iter()
                .any(|grant| grant.key == "speech_stream")
        );
    }

    #[test]
    fn every_permission_reads_as_the_author_guide_says() {
        let manifest = Manifest::parse(&format!("memory_mb = 128\n{BASE}")).unwrap();
        let tools = ["prices".to_string(), "cheapest_window".to_string()];
        assert_eq!(
            texts(&grants(&manifest, Some(&tools))),
            vec![
                "Connect to api.energidataservice.dk",
                "Connect to any address under tibber.com",
                "Reach the device at the address you enter under Address, on your home network",
                "Reach 192.168.1.50 on your home network",
                "Reach any of 256 addresses on your home network (192.168.20.0/24)",
                "Add devices to your home. They appear in rooms, in rules and by voice.",
                "Offer 2 tools to the assistant: prices, cheapest_window",
                "See every device in your home change: lights, doors, motion",
                "See button presses and what other devices report",
                "Know when the house becomes occupied or empty",
                "Speak through your satellites, even during quiet hours",
                "Adds a page to the menu: Electricity",
                "Adds a card to the front page: Electricity now",
                "Adds a card to the Living room: Air quality",
                "Use up to 128 MB of memory",
            ]
        );
    }

    #[test]
    fn a_widget_placed_by_a_setting_says_which() {
        let manifest = Manifest::parse(&format!(
            "{BASE}
[[settings]]
key = \"room\"
label = \"Printer room\"
type = \"room\"

[[ui.widgets]]
id = \"printer\"
title = \"Printer\"
room = \"setting:room\"
"
        ))
        .unwrap();
        assert!(
            texts(&grants(&manifest, None))
                .contains(&"Adds a card to the room its \"Printer room\" setting names: Printer")
        );
    }

    #[test]
    fn one_tool_and_unknown_tools_are_worded_for_what_is_known() {
        let manifest = Manifest::parse(BASE).unwrap();
        let one = ["prices".to_string()];
        let worded = |tools| {
            grants(&manifest, tools)
                .into_iter()
                .find(|g| g.key == "tools")
                .unwrap()
                .text
        };
        assert_eq!(worded(Some(&one)), "Offer 1 tool to the assistant: prices");
        assert_eq!(worded(None), "Offer tools to the assistant");
    }

    #[test]
    fn brokers_say_where_and_how_their_certificate_is_trusted() {
        let manifest = Manifest::parse(
            r#"
id = "printer"
name = "Printer"
version = "0.1.0"
description = "x"
authors = ["a"]
min_niles = "0.2.0"
interface = "0.2"

[permissions]
mqtt = [
    { name = "printer", host = "setting:address", tls = "trust-device" },
    { name = "plugs", host = "192.168.1.20", tls = "none" },
    { name = "cloud", host = "broker.example.com", port = 8884 },
]

[[settings]]
key = "address"
label = "Printer address"
type = "string"
"#,
        )
        .unwrap();
        assert_eq!(
            texts(&grants(&manifest, None)),
            vec![
                "Connect to the MQTT broker at the address you enter under Printer address, \
                 on your home network, trusting the certificate it shows the first time",
                "Connect to the MQTT broker at 192.168.1.20, on your home network, unencrypted",
                "Connect to the MQTT broker at broker.example.com, port 8884",
            ]
        );
        let keys: Vec<String> = grants(&manifest, None).into_iter().map(|g| g.key).collect();
        assert_eq!(keys[0], "mqtt:printer:setting:address:8883:trust-device");
    }

    #[test]
    fn websocket_grants_are_plain_and_explicit_about_receive_access() {
        let manifest = Manifest::parse(
            r#"
id = "feed"
name = "Feed"
version = "0.1.0"
description = "x"
authors = ["a"]
min_niles = "0.4.0"
interface = "0.4"

[permissions]
websocket = [
    { name = "public", host = "stream.example.com", tls = true },
    { name = "device", host = "192.168.1.50", tls = false },
]
"#,
        )
        .unwrap();
        assert_eq!(
            texts(&grants(&manifest, None)),
            vec![
                "Keep a connection open to stream.example.com, port 443, and receive what it sends over a TLS connection with certificate verification",
                "Keep a connection open to 192.168.1.50, port 80, and receive what it sends over an unencrypted connection on your home network",
            ]
        );
    }

    #[test]
    fn websocket_keyed_diff_reports_added_widened_and_unchanged_grants() {
        let parse = |websocket: &str| {
            Manifest::parse(&format!(
                "id = \"feed\"\nname = \"Feed\"\nversion = \"0.1.0\"\ndescription = \"x\"\nauthors = [\"a\"]\nmin_niles = \"0.4.0\"\ninterface = \"0.4\"\n[permissions]\nwebsocket = [{websocket}]\n"
            ))
            .unwrap()
        };
        let before = parse(
            "{ name = \"events\", host = \"feed.example.test\", port = 8443, tls = true }, { name = \"local\", host = \"127.0.0.1\", port = 8080 }",
        );
        let after = parse(
            "{ name = \"events\", host = \"feed.example.test\", port = 9443, tls = true }, { name = \"local\", host = \"127.0.0.1\", port = 8080 }, { name = \"updates\", host = \"updates.example.test\", tls = true }",
        );
        let change = diff(&grants(&before, None), &grants(&after, None));
        assert_eq!(change.added.len(), 2);
        assert_eq!(change.removed.len(), 1);
        assert!(
            change
                .added
                .iter()
                .any(|text| text.contains("feed.example.test, port 9443"))
        );
        assert!(
            change
                .added
                .iter()
                .any(|text| text.contains("updates.example.test, port 443"))
        );
        assert!(change.removed[0].contains("feed.example.test, port 8443"));

        let changed = parse(
            "{ name = \"events\", host = \"other.example.test\", port = 8443, tls = false }, { name = \"local\", host = \"127.0.0.1\", port = 8080 }",
        );
        let change = diff(&grants(&before, None), &grants(&changed, None));
        assert_eq!(change.added.len(), 1);
        assert_eq!(change.removed.len(), 1);
        assert!(change.added[0].contains("other.example.test, port 8443"));
        assert!(change.added[0].contains("unencrypted"));
        assert!(change.removed[0].contains("feed.example.test, port 8443"));
        assert!(change.removed[0].contains("TLS"));
    }

    #[test]
    fn nothing_asked_for_is_nothing_shown() {
        let manifest = Manifest::parse(
            "id = \"quiet\"\nname = \"Quiet\"\nversion = \"0.1.0\"\ndescription = \"x\"\n\
             authors = [\"a\"]\nmin_niles = \"0.2.0\"\ninterface = \"0.1\"\n",
        )
        .unwrap();
        assert!(grants(&manifest, None).is_empty());
    }

    #[test]
    fn the_difference_is_in_what_it_may_do_not_in_the_wording() {
        let before = Manifest::parse(BASE).unwrap();
        let after = Manifest::parse(
            &BASE
                .replace("\"*.tibber.com\"", "\"api.tibber.com\"")
                .replace("notify = \"urgent\"", "notify = true")
                .replace("label = \"Electricity\"", "label = \"Power\""),
        )
        .unwrap();
        let tools = ["prices".to_string()];
        let more_tools = ["prices".to_string(), "cheapest".to_string()];
        let diff = diff(
            &grants(&before, Some(&tools)),
            &grants(&after, Some(&more_tools)),
        );
        assert_eq!(
            diff.added,
            vec!["Connect to api.tibber.com", "Speak through your satellites"]
        );
        assert_eq!(
            diff.removed,
            vec![
                "Connect to any address under tibber.com",
                "Speak through your satellites, even during quiet hours"
            ]
        );
    }
}
