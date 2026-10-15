//! What a plugin asks for, in the words the approval screen uses.
//!
//! The wording is `docs/integrations/permissions.md`'s, line for line.
//! Each line is a [`Grant`] with a key that names the permission itself
//! rather than how it is worded, so an update's difference is what it
//! may now do that it could not before — not a page label that changed,
//! or a tool that was added under a `tools` permission it already had.

use crate::manifest::{DEFAULT_MEMORY_MB, EventPermission, Manifest, Notify};
use serde::Serialize;
use std::collections::HashSet;

/// Always said last, whatever the plugin asks for.
pub const CANNOT: &str = "It cannot see your conversations, the people in your house, \
     other integrations' passwords or Niles's database.";

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
            EventPermission::DeviceAction => ("device_action", "See button presses"),
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
    if p.ui {
        for entry in &manifest.ui.menu {
            add(
                format!("ui:menu:{}", entry.id),
                format!("Adds a page to the menu: {}", entry.label),
            );
        }
        for widget in &manifest.ui.widgets {
            let place = match &widget.room {
                Some(room) => format!("the {}", room_name(room)),
                None => "the front page".to_string(),
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
                "See button presses",
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
