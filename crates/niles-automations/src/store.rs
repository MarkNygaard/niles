//! The household's rules: made by voice or in the app, kept in order,
//! and handed to a sink to be written down after every change.
//!
//! One document holding the whole set, as scenes and the shopping list
//! are kept — a handful of rules, read together, whose order is part of
//! the record (the later of two conflicting rules wins).

use crate::error::{Error, Result};
use crate::rule::Rule;
use niles_config::AutomationRuleDto;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// Where the document goes after a change. Fire and forget: the binary
/// queues it for Postgres.
pub trait AutomationPersistence: Send + Sync {
    fn store(&self, document: &str);
}

/// The source prefix a rule's bare `room/name` devices are read with.
pub const DEFAULT_SOURCE: &str = "z2m";

pub struct AutomationStore {
    rules: Mutex<Vec<AutomationRuleDto>>,
    sink: Option<Arc<dyn AutomationPersistence>>,
    /// Bumped after each change, so the engine can take the new set.
    changed: watch::Sender<u64>,
}

impl Default for AutomationStore {
    fn default() -> Self {
        Self {
            rules: Mutex::new(Vec::new()),
            sink: None,
            changed: watch::Sender::new(0),
        }
    }
}

impl AutomationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a document a sink was handed.
    pub fn from_json(document: &str) -> serde_json::Result<Self> {
        let store = Self::default();
        *store.lock() = serde_json::from_str(document)?;
        Ok(store)
    }

    pub fn with_sink(mut self, sink: Arc<dyn AutomationPersistence>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Every rule, in the order they run.
    pub fn list(&self) -> Vec<AutomationRuleDto> {
        self.lock().clone()
    }

    pub fn get(&self, id: &str) -> Option<AutomationRuleDto> {
        self.lock().iter().find(|r| r.id == id).cloned()
    }

    /// The rules the engine runs. One that no longer parses — a device
    /// renamed out from under it, say — is left out and logged, not
    /// allowed to take the others down with it.
    pub fn runnable(&self) -> Vec<Rule> {
        self.lock()
            .iter()
            .filter_map(|dto| match Rule::from_dto(dto, DEFAULT_SOURCE) {
                Ok(rule) => Some(rule),
                Err(e) => {
                    tracing::warn!("automation '{}' is not running: {e}", dto.id);
                    None
                }
            })
            .collect()
    }

    /// Add a rule at the end, named after its description. Whatever
    /// `id` it came with is replaced.
    pub fn create(&self, mut rule: AutomationRuleDto) -> Result<AutomationRuleDto> {
        let mut rules = self.lock();
        rule.id = unique_id(&rule.description, &rules);
        Rule::from_dto(&rule, DEFAULT_SOURCE)?;
        rules.push(rule.clone());
        self.saved(&rules);
        Ok(rule)
    }

    /// Replace the rule with `id`, keeping its place in the order.
    pub fn update(&self, id: &str, mut rule: AutomationRuleDto) -> Result<AutomationRuleDto> {
        let mut rules = self.lock();
        let at = position(&rules, id)?;
        rule.id = id.to_string();
        Rule::from_dto(&rule, DEFAULT_SOURCE)?;
        rules[at] = rule.clone();
        self.saved(&rules);
        Ok(rule)
    }

    /// Pause (`false`) or resume (`true`) the rule with `id`.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<AutomationRuleDto> {
        let mut rules = self.lock();
        let at = position(&rules, id)?;
        rules[at].enabled = enabled;
        let rule = rules[at].clone();
        self.saved(&rules);
        Ok(rule)
    }

    pub fn remove(&self, id: &str) -> Result<AutomationRuleDto> {
        let mut rules = self.lock();
        let at = position(&rules, id)?;
        let gone = rules.remove(at);
        self.saved(&rules);
        Ok(gone)
    }

    /// Wakes after every change.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn saved(&self, rules: &[AutomationRuleDto]) {
        if let Some(sink) = &self.sink {
            match serde_json::to_string(rules) {
                Ok(document) => sink.store(&document),
                Err(e) => tracing::error!("automations could not be written down: {e}"),
            }
        }
        self.changed.send_modify(|n| *n += 1);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<AutomationRuleDto>> {
        self.rules.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn position(rules: &[AutomationRuleDto], id: &str) -> Result<usize> {
    rules
        .iter()
        .position(|r| r.id == id)
        .ok_or_else(|| Error::NotFound { id: id.to_string() })
}

/// "Office lights off when the garden door opens" →
/// `office-lights-off-when-the-garden-door-opens`, with `-2`, `-3`… for
/// a name already taken.
fn unique_id(description: &str, rules: &[AutomationRuleDto]) -> String {
    let mut slug = String::new();
    for c in description.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let base = if slug.is_empty() { "automation" } else { slug };
    let taken = |id: &str| rules.iter().any(|r| r.id == id);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .expect("an unbounded range always has a free number")
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_config::{ActionDto, TriggerDto};

    fn garden_door(description: &str) -> AutomationRuleDto {
        AutomationRuleDto {
            id: String::new(),
            description: description.into(),
            enabled: true,
            trigger: TriggerDto::Contact {
                device: "office/door".into(),
                open: Some(true),
            },
            conditions: vec![],
            actions: vec![ActionDto::SetRoom {
                room: "office".into(),
                on: Some(false),
                brightness: None,
                kelvin: None,
            }],
        }
    }

    struct Keep(Mutex<Option<String>>);
    impl AutomationPersistence for Keep {
        fn store(&self, document: &str) {
            *self.0.lock().unwrap() = Some(document.to_string());
        }
    }

    #[test]
    fn names_a_rule_after_its_description() {
        let store = AutomationStore::new();
        let made = store
            .create(garden_door("Office lights off, garden door!"))
            .unwrap();
        assert_eq!(made.id, "office-lights-off-garden-door");
        let again = store
            .create(garden_door("Office lights off, garden door!"))
            .unwrap();
        assert_eq!(again.id, "office-lights-off-garden-door-2");
        assert_eq!(store.create(garden_door("")).unwrap().id, "automation");
    }

    #[test]
    fn refuses_a_rule_that_does_nothing() {
        let store = AutomationStore::new();
        let mut rule = garden_door("nothing");
        rule.actions.clear();
        assert!(matches!(store.create(rule), Err(Error::NoActions { .. })));
        assert!(store.list().is_empty());
    }

    #[test]
    fn keeps_a_rule_in_its_place_when_it_changes() {
        let store = AutomationStore::new();
        store.create(garden_door("first")).unwrap();
        store.create(garden_door("second")).unwrap();
        let mut changed = garden_door("first, changed");
        changed.enabled = false;
        store.update("first", changed).unwrap();
        let ids: Vec<_> = store.list().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["first", "second"]);
        assert!(!store.get("first").unwrap().enabled);
        assert!(matches!(
            store.update("gone", garden_door("x")),
            Err(Error::NotFound { .. })
        ));
    }

    #[test]
    fn writes_down_every_change_and_reads_it_back() {
        let sink = Arc::new(Keep(Mutex::new(None)));
        let store = AutomationStore::new().with_sink(sink.clone());
        let changes = store.subscribe();
        store.create(garden_door("garden door")).unwrap();
        store.set_enabled("garden-door", false).unwrap();
        assert!(changes.has_changed().unwrap());

        let document = sink.0.lock().unwrap().clone().unwrap();
        let restored = AutomationStore::from_json(&document).unwrap();
        assert_eq!(restored.list(), store.list());
        assert!(!restored.get("garden-door").unwrap().enabled);

        store.remove("garden-door").unwrap();
        assert_eq!(sink.0.lock().unwrap().as_deref(), Some("[]"));
    }

    #[test]
    fn runs_only_the_rules_that_still_parse() {
        let store =
            AutomationStore::from_json(r#"[{"id":"bad","trigger":{"type":"contact","device":"no slash"},"actions":[{"do":"set_room","room":"office","on":false}]}]"#)
                .unwrap();
        store.create(garden_door("good")).unwrap();
        let ids: Vec<_> = store.runnable().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["good"]);
    }
}
