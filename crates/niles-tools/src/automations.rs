//! Automation tools — `propose_automation`, `create_automation`,
//! `list_automations`, `pause_automation`, `remove_automation` and
//! `automation_runs`.
//!
//! A rule made by voice is read back before it is saved: the model
//! proposes it, says the sentence, and creates it only on a yes. A
//! misheard rule would otherwise go live in a house nobody is looking
//! at. Once saved it is on the Automations page like any other.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_automations::store::DEFAULT_SOURCE;
use niles_automations::{AutomationStore, Measure, Rule, RunLog, sentence};
use niles_config::{ActionDto, AutomationRuleDto, ConditionDto, TriggerDto, TriggerKind};
use niles_core::{DeviceId, DeviceRegistry, RoomName};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The rule last read back and not yet answered. A yes to it is saved
/// by the dispatcher itself, not left to the model: a model that
/// proposed the rule again and then said "saved" left the household
/// with nothing, and believing otherwise.
#[derive(Default)]
pub struct Proposals {
    pending: Mutex<Option<(AutomationRuleDto, Instant)>>,
    /// When a rule was last talked about without one to say yes to:
    /// a question back, or a read-back with something wrong in it.
    talk: Mutex<Option<Instant>>,
}

impl Proposals {
    /// How long a read-back waits for its answer.
    const FRESH: Duration = Duration::from_secs(3 * 60);

    pub fn new() -> Self {
        Self::default()
    }

    fn offer(&self, rule: AutomationRuleDto) {
        *self.lock() = Some((rule, Instant::now()));
    }

    /// A rule is being worked out: what is said next belongs to it,
    /// even when it does not say "automation" again — "half an hour
    /// after the sun has set".
    pub fn discussed(&self) {
        *self.talk_lock() = Some(Instant::now());
    }

    /// Whether a rule is being worked out, or waiting for its answer.
    pub fn in_talk(&self) -> bool {
        self.pending()
            || self
                .talk_lock()
                .is_some_and(|at| at.elapsed() < Self::FRESH)
    }

    fn talk_lock(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.talk.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The rule waiting for an answer, if one still is.
    pub fn pending(&self) -> bool {
        self.lock()
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < Self::FRESH)
    }

    /// Take the rule waiting for an answer.
    pub fn take(&self) -> Option<AutomationRuleDto> {
        self.talk_lock().take();
        self.lock()
            .take()
            .filter(|(_, at)| at.elapsed() < Self::FRESH)
            .map(|(rule, _)| rule)
    }

    pub fn clear(&self) {
        self.talk_lock().take();
        self.lock().take();
    }

    /// The rule read back since `since`, still waiting: what a turn
    /// that began then asked to save.
    pub fn offered_since(&self, since: Instant) -> Option<AutomationRuleDto> {
        self.lock()
            .as_ref()
            .filter(|(_, at)| *at >= since)
            .map(|(rule, _)| rule.clone())
    }

    /// `rule` was answered elsewhere — a button in the app — so a yes
    /// said next is not taken for it too.
    pub fn settled(&self, rule: &AutomationRuleDto) {
        let mut pending = self.lock();
        if pending.as_ref().is_some_and(|(waiting, _)| waiting == rule) {
            pending.take();
            self.talk_lock().take();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(AutomationRuleDto, Instant)>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Whether `said` asks for a rule to be made or changed: worth the
/// slower, better model, which gets the shape of one right the first
/// time. Narrower than the tool groups' "when" and "if" on purpose —
/// "when is my next meeting" is not a rule.
pub fn about_automations(said: &str) -> bool {
    let words: String = said
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let padded = format!(" {words} ");
    let has = |phrase: &str| padded.contains(&format!(" {phrase} "));
    const RULE: &[&str] = &[
        "automation",
        "automations",
        "automate",
        "rule",
        "rules",
        "whenever",
        "every time",
        "each time",
        "as soon as",
    ];
    if RULE.iter().any(|p| has(p)) {
        return true;
    }
    // "Remind me to close the curtains after sunset" is a rule; "remind
    // me in ten minutes" is a timer.
    const MOMENT: &[&str] = &["sunset", "sunrise", "sun", "every", "when", "whenever"];
    if has("remind me") && MOMENT.iter().any(|p| has(p)) {
        return true;
    }
    // "Wake me up seven hours after I go to bed" is a rule; "wake me at
    // six" is an alarm.
    const LATER: &[&str] = &["after", "when", "whenever", "every"];
    (has("wake me") || has("wake us")) && LATER.iter().any(|p| has(p))
}

/// A plain yes or no to a question, and nothing else: `Some(true)` for
/// "yes", "yes please", "go ahead, Niles"; `Some(false)` for "no",
/// "cancel"; `None` for anything that says more than that.
pub fn answer(said: &str) -> Option<bool> {
    let words: Vec<String> = said
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’')
        .filter(|w| !w.is_empty())
        .map(|w| w.replace('’', "'"))
        .filter(|w| {
            !matches!(
                w.as_str(),
                "niles" | "nyles" | "please" | "thanks" | "thank" | "you"
            )
        })
        .collect();
    let said = words.join(" ");
    const YES: &[&str] = &[
        "yes",
        "yeah",
        "yep",
        "yup",
        "sure",
        "ok",
        "okay",
        "do it",
        "go ahead",
        "save it",
        "yes do it",
        "yes save it",
        "that's right",
        "correct",
        "sounds good",
        "ja",
        "ja tak",
        "gem den",
        "yes go ahead",
        "absolutely",
        "of course",
    ];
    const NO: &[&str] = &[
        "no",
        "nope",
        "don't",
        "cancel",
        "never mind",
        "nevermind",
        "no don't",
        "nej",
        "no thanks",
        "forget it",
        "stop",
    ];
    if YES.contains(&said.as_str()) {
        Some(true)
    } else if NO.contains(&said.as_str()) {
        Some(false)
    } else {
        None
    }
}

/// The rule as the model writes it — the same JSON the app sends.
fn rule_schema() -> Value {
    let device = json!({
        "type": "string",
        "description": "A device as room/name, e.g. office/door, office/go."
    });
    let duration = json!({
        "type": "object",
        "description": "How long, as hours/minutes/seconds.",
        "properties": {
            "hours": { "type": "integer", "minimum": 0 },
            "minutes": { "type": "integer", "minimum": 0 },
            "seconds": { "type": "integer", "minimum": 0 }
        }
    });
    let measure = json!({
        "type": "string",
        "description": "temperature, humidity, battery, brightness, illuminance (lux; below about 30 is dim), or any reading name the device reports, e.g. price, power, energy, co2 — get_device_state lists a device's readings and their units."
    });
    let days = json!({
        "type": "array",
        "items": { "type": "string", "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] }
    });
    let condition = json!({
        "type": "object",
        "description": "kind=time_of_day: after/before HH:MM (may wrap midnight), optional days. kind=sun: after and/or before sunrise|sunset, with after_offset_minutes/before_offset_minutes (after sunset + before sunrise = while it is dark). kind=device_is: a device on/off. kind=contact_is: a door open/closed. kind=motion_is: a motion sensor seeing somebody (detected=true) or not. kind=numeric: a number the device reports (see measure) above and/or below a value. kind=presence_is: state home|away. kind=triggered_by: which of the rule's triggers (by id) started it — for a rule with several triggers doing different things.",
        "properties": {
            "kind": { "type": "string", "enum": ["time_of_day", "sun", "device_is", "contact_is", "motion_is", "numeric", "presence_is", "triggered_by"] },
            "after": { "type": "string" },
            "before": { "type": "string" },
            "after_offset_minutes": { "type": "integer" },
            "before_offset_minutes": { "type": "integer" },
            "days": days,
            "device": device,
            "on": { "type": "boolean" },
            "open": { "type": "boolean" },
            "detected": { "type": "boolean" },
            "measure": measure,
            "above": { "type": "number" },
            "below": { "type": "number" },
            "state": { "type": "string", "enum": ["home", "away"] },
            "triggers": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["kind"]
    });
    let action = json!({
        "type": "object",
        "description": "do=set_room: every light in a room (room, on, brightness 0-100, kelvin 2000-6500). do=set_device: one device. do=set_blinds: blinds/curtains to a position (0 closed, 100 open), one (device) or every one in a room (room). do=notify: say body in a room (a room with a satellite). do=wake_up: the morning routine, now — lights that are off come on at 0% and ramp up over minutes (1-120, default 45) to brightness (default 100), then hold there; colour follows the curve unless kelvin (the end warmth) is given, and start_kelvin with it warms from one to the other, e.g. 2200 to 4000 for a sunrise; rooms and devices pick the lights (every light in the house when neither is given), except leaves some out; for 'wake me up', usually after a delay. do=delay: wait hours/minutes/seconds, then go on. do=if: conditions, then (actions), else (actions) — at most 3 deep.",
        "properties": {
            "do": { "type": "string", "enum": ["set_room", "set_device", "set_blinds", "notify", "wake_up", "delay", "if"] },
            "rooms": { "type": "array", "items": { "type": "string" } },
            "devices": { "type": "array", "items": device.clone() },
            "except": { "type": "array", "items": device.clone() },
            "room": { "type": "string" },
            "device": device,
            "on": { "type": "boolean" },
            "brightness": { "type": "integer", "minimum": 0, "maximum": 100 },
            "kelvin": { "type": "integer", "minimum": 2000, "maximum": 6500 },
            "start_kelvin": { "type": "integer", "minimum": 2000, "maximum": 6500 },
            "position": { "type": "integer", "minimum": 0, "maximum": 100 },
            "body": { "type": "string" },
            "hours": { "type": "integer", "minimum": 0 },
            "minutes": { "type": "integer", "minimum": 0 },
            "seconds": { "type": "integer", "minimum": 0 },
            "conditions": { "type": "array", "items": condition },
            "then": { "type": "array", "items": { "type": "object" } },
            "else": { "type": "array", "items": { "type": "object" } }
        },
        "required": ["do"]
    });
    json!({
        "type": "object",
        "properties": {
            "description": {
                "type": "string",
                "description": "A short name for the rule, e.g. 'Office lights off when the garden door opens'."
            },
            "triggers": {
                "type": "array",
                "minItems": 1,
                "description": "What starts it; one or more. Give each an id (t1, t2…) when conditions need to tell them apart. A moment relative to the sun ('30 minutes after sunset') is ONE sun trigger with offset_minutes 30 — not a sunset trigger with a condition, which would never hold at sunset. type=contact: a door or window opens (open=true) or closes (false). type=motion: a motion sensor sees somebody (detected=true) or stops (false); lights on with motion and off again is TWO triggers — motion detected=true, and motion detected=false with `for` ('no motion for 5 minutes') — told apart by triggered_by. type=device_state: a device turns on/off. type=numeric: a number the device reports (see measure) goes above/below a value — a price, a power draw, a temperature. type=device_action: a button press. type=presence: someone comes home (state=home) or everyone leaves (away). type=time: at HH:MM, optional days. type=sun: sunrise|sunset with offset_minutes (negative: before). type=timer_fired. contact, motion, device_state, numeric and presence take `for`: fire only once it has held that long ('open for 10 minutes').",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "type": { "type": "string", "enum": ["contact", "motion", "device_state", "numeric", "device_action", "presence", "time", "sun", "timer_fired"] },
                        "device": device,
                        "room": { "type": "string" },
                        "open": { "type": "boolean" },
                        "detected": { "type": "boolean" },
                        "on": { "type": "boolean" },
                        "measure": measure,
                        "above": { "type": "number" },
                        "below": { "type": "number" },
                        "action": { "type": "string" },
                        "state": { "type": "string", "enum": ["home", "away"] },
                        "at": { "type": "string", "description": "HH:MM" },
                        "days": days,
                        "event": { "type": "string", "enum": ["sunrise", "sunset"] },
                        "offset_minutes": { "type": "integer" },
                        "name": { "type": "string" },
                        "for": duration
                    },
                    "required": ["type"]
                }
            },
            "conditions": {
                "type": "array",
                "description": "Optional; all must hold for the actions to run.",
                "items": condition
            },
            "actions": {
                "type": "array",
                "minItems": 1,
                "description": "What it does, in order.",
                "items": action
            }
        },
        "required": ["description", "triggers", "actions"]
    })
}

/// Read the model's rule. It is not trusted to leave out what the app
/// would: an `id` it made up is replaced, and `enabled` starts on.
fn read_rule(tool: &str, args: &Value) -> Result<AutomationRuleDto> {
    let mut body = args.clone();
    let Some(object) = body.as_object_mut() else {
        return Err(invalid(tool, "the rule must be an object".into()));
    };
    object.insert("id".into(), Value::String(String::new()));
    object.entry("enabled").or_insert(Value::Bool(true));
    object.entry("conditions").or_insert(json!([]));
    let mut rule: AutomationRuleDto =
        serde_json::from_value(body).map_err(|e| invalid(tool, format!("not a rule: {e}")))?;
    lift_moment(&mut rule);
    Ok(rule)
}

/// A rule with no trigger, whose one time or sun condition only says
/// *after* something — "after sunset, plus thirty minutes" — means at
/// that moment. The model wrote it so twice in an evening ("remind me
/// to close the curtains 30 minutes after sunset"); read it as meant
/// rather than refuse it.
fn lift_moment(rule: &mut AutomationRuleDto) {
    if !rule.triggers.is_empty() {
        return;
    }
    let moments: Vec<usize> = rule
        .conditions
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            matches!(
                c,
                ConditionDto::Sun {
                    after: Some(_),
                    before: None,
                    ..
                } | ConditionDto::TimeOfDay {
                    after: Some(_),
                    before: None,
                    ..
                }
            )
        })
        .map(|(i, _)| i)
        .collect();
    let [only] = moments[..] else { return };
    let kind = match rule.conditions.remove(only) {
        ConditionDto::Sun {
            after: Some(event),
            after_offset_minutes,
            ..
        } => TriggerKind::Sun {
            event,
            offset_minutes: after_offset_minutes,
        },
        ConditionDto::TimeOfDay {
            after: Some(at),
            days,
            ..
        } => TriggerKind::Time { at, days },
        _ => return,
    };
    rule.triggers.push(TriggerDto { id: None, kind });
}

/// What the rule names that this house does not have — a misheard
/// "office" as "offers", or a reading the device never reports.
/// Checked here rather than in the store because the app may well save
/// a rule for a device not yet paired.
fn unknown(rule: &AutomationRuleDto, registry: &DeviceRegistry) -> Vec<String> {
    let mut devices: Vec<&str> = Vec::new();
    let mut rooms: Vec<&str> = Vec::new();
    let mut measured: Vec<(&str, &str)> = Vec::new();
    for trigger in &rule.triggers {
        match &trigger.kind {
            TriggerKind::Numeric {
                device, measure, ..
            } => {
                devices.push(device);
                measured.push((device, measure));
            }
            TriggerKind::Contact { device, .. }
            | TriggerKind::Motion { device, .. }
            | TriggerKind::DeviceAction { device, .. } => devices.push(device),
            TriggerKind::DeviceState { device, room, .. } => {
                devices.extend(device.as_deref());
                rooms.extend(room.as_deref());
            }
            _ => {}
        }
    }
    fn in_conditions<'a>(
        conditions: &'a [ConditionDto],
        devices: &mut Vec<&'a str>,
        measured: &mut Vec<(&'a str, &'a str)>,
    ) {
        for condition in conditions {
            match condition {
                ConditionDto::Numeric {
                    device, measure, ..
                } => {
                    devices.push(device);
                    measured.push((device, measure));
                }
                ConditionDto::DeviceIs { device, .. }
                | ConditionDto::ContactIs { device, .. }
                | ConditionDto::MotionIs { device, .. } => devices.push(device),
                _ => {}
            }
        }
    }
    fn in_actions<'a>(
        actions: &'a [ActionDto],
        devices: &mut Vec<&'a str>,
        lit: &mut Vec<&'a str>,
        measured: &mut Vec<(&'a str, &'a str)>,
    ) {
        for action in actions {
            match action {
                ActionDto::SetDevice { device, .. } => devices.push(device),
                ActionDto::SetRoom { room, .. } => lit.push(room),
                ActionDto::SetBlinds { device, .. } => devices.extend(device.as_deref()),
                ActionDto::WakeUp {
                    rooms,
                    devices: named,
                    except,
                    ..
                } => {
                    lit.extend(rooms.iter().map(String::as_str));
                    devices.extend(named.iter().map(String::as_str));
                    devices.extend(except.iter().map(String::as_str));
                }
                ActionDto::If {
                    conditions,
                    then,
                    otherwise,
                } => {
                    in_conditions(conditions, devices, measured);
                    in_actions(then, devices, lit, measured);
                    in_actions(otherwise, devices, lit, measured);
                }
                _ => {}
            }
        }
    }
    in_conditions(&rule.conditions, &mut devices, &mut measured);
    in_actions(&rule.actions, &mut devices, &mut rooms, &mut measured);
    devices.sort_unstable();
    devices.dedup();
    rooms.sort_unstable();
    rooms.dedup();
    measured.sort_unstable();
    measured.dedup();
    let find = |device: &str| {
        let full = if device.contains(':') {
            device.to_string()
        } else {
            format!("{DEFAULT_SOURCE}:{device}")
        };
        DeviceId::parse(&full).ok().and_then(|id| registry.get(&id))
    };
    let mut missing = Vec::new();
    for device in devices {
        if find(device).is_none() {
            missing.push(format!("no device called {device}"));
        }
    }
    // Any valid name parses as a reading, so "powr" for "power" would
    // be a rule that never fires and never says why. A missing device
    // has been said above.
    for (device, measure) in measured {
        let (Some(found), Some(Measure::Reading(name))) = (find(device), Measure::parse(measure))
        else {
            continue;
        };
        // Declared by the source counts as much as already reported: a
        // plug that has drawn nothing since Niles started still has a
        // `power` reading.
        if found.state.readings.contains_key(&name) || found.reading_units.contains_key(&name) {
            continue;
        }
        let mut has: Vec<&str> = found
            .state
            .readings
            .keys()
            .chain(found.reading_units.keys())
            .map(String::as_str)
            .collect();
        has.sort_unstable();
        has.dedup();
        missing.push(if has.is_empty() {
            format!("{device} reports no reading called {name}, nor any other")
        } else {
            format!(
                "{device} reports no reading called {name}; it reports {}",
                has.join(", ")
            )
        });
    }
    for room in rooms {
        let lit = RoomName::parse(room)
            .map(|r| registry.list_room(&r).iter().any(|d| d.is_light()))
            .unwrap_or(false);
        if !lit {
            missing.push(format!("no lights in a room called {room}"));
        }
    }
    missing
}

/// Whether two rules start on any of the same kind of trigger for the
/// same thing — "you already have one for the garden door".
fn overlaps(a: &[TriggerDto], b: &[TriggerDto]) -> bool {
    a.iter().any(|x| b.iter().any(|y| x.kind == y.kind))
}

pub struct ProposeAutomation {
    store: Arc<AutomationStore>,
    registry: Arc<DeviceRegistry>,
    proposals: Arc<Proposals>,
    tz: chrono_tz::Tz,
}

#[async_trait]
impl Tool for ProposeAutomation {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "propose_automation".into(),
            description: "First step of making an automation — 'when the office door opens, \
                turn off the office lights', 'turn the lights off five minutes after the door \
                closes', 'tell me if the garden door has been open for ten minutes', 'turn on the \
                porch light at sunset'. Saves nothing. Returns the rule as a sentence: read it to \
                the person and ask whether to save it ('When the office door opens, I'll turn \
                off the office lights. Shall I?'). Call create_automation with the same rule \
                only after they say yes. If it returns problems, say them instead and ask what \
                they meant."
                .into(),
            parameters: rule_schema(),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let rule = read_rule("propose_automation", &args)?;
        let mut problems = unknown(&rule, &self.registry);
        match Rule::from_dto(
            &AutomationRuleDto {
                id: "proposed".into(),
                ..rule.clone()
            },
            DEFAULT_SOURCE,
        ) {
            Err(e) => problems.push(e.to_string()),
            Ok(parsed) => {
                if parsed.uses_sun() && self.store.location().is_none() {
                    problems.push("the sun needs the house's location, which is not set".into());
                }
                if parsed.never_runs(
                    &self.registry,
                    chrono::Utc::now(),
                    self.tz,
                    self.store.location(),
                ) {
                    problems.push(
                        "it could never run: when its trigger fires, its time or sun conditions \
                         do not hold yet. For a time after sunset, use a sun trigger with \
                         offset_minutes instead of a condition"
                            .into(),
                    );
                }
            }
        }
        if problems.is_empty() {
            self.proposals.offer(rule.clone());
        } else {
            self.proposals.clear();
        }
        let similar: Vec<String> = self
            .store
            .list()
            .into_iter()
            .filter(|r| overlaps(&r.triggers, &rule.triggers))
            .map(|r| sentence(&r))
            .collect();
        Ok(json!({
            "sentence": sentence(&rule),
            "problems": problems,
            "existing_with_the_same_trigger": similar,
            "saved": false,
            "next": if problems.is_empty() {
                "NOT saved yet. Read the sentence back and ask whether to save it. Do not say it is saved."
            } else {
                "NOT saved. Say what is wrong and ask what they meant."
            },
        }))
    }
}

pub struct CreateAutomation {
    store: Arc<AutomationStore>,
    registry: Arc<DeviceRegistry>,
    proposals: Arc<Proposals>,
}

#[async_trait]
impl Tool for CreateAutomation {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "create_automation".into(),
            description: "Save an automation the person has just said yes to, after \
                propose_automation read it back. Never call this without that yes. Pass the \
                same rule propose_automation was given."
                .into(),
            parameters: rule_schema(),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let rule = read_rule("create_automation", &args)?;
        let problems = unknown(&rule, &self.registry);
        if !problems.is_empty() {
            return Err(invalid("create_automation", problems.join("; ")));
        }
        let made = self
            .store
            .create(rule)
            .map_err(|e| invalid("create_automation", e.to_string()))?;
        self.proposals.clear();
        Ok(json!({ "saved": true, "id": made.id, "sentence": sentence(&made) }))
    }
}

pub struct ListAutomations {
    store: Arc<AutomationStore>,
}

#[async_trait]
impl Tool for ListAutomations {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "list_automations".into(),
            description: "The household's automations: 'what automations do I have?'. Also \
                the way to find one's id before pausing, removing or asking about it. Say them \
                as their sentences."
                .into(),
            parameters: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let rules: Vec<Value> = self
            .store
            .list()
            .into_iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "name": r.description,
                    "sentence": sentence(&r),
                    "paused": !r.enabled,
                })
            })
            .collect();
        Ok(json!({ "automations": rules }))
    }
}

pub struct PauseAutomation {
    store: Arc<AutomationStore>,
}

#[async_trait]
impl Tool for PauseAutomation {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "pause_automation".into(),
            description: "Pause an automation ('pause the garden door automation') or switch a \
                paused one back on (paused=false). Find its id with list_automations."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "paused": { "type": "boolean", "description": "true to pause, false to resume." }
                },
                "required": ["id", "paused"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let id = required("pause_automation", &args, "id")?;
        let paused = args.get("paused").and_then(Value::as_bool).unwrap_or(true);
        let rule = self
            .store
            .set_enabled(id, !paused)
            .map_err(|e| invalid("pause_automation", e.to_string()))?;
        Ok(json!({ "id": rule.id, "paused": !rule.enabled, "sentence": sentence(&rule) }))
    }
}

pub struct RemoveAutomation {
    store: Arc<AutomationStore>,
}

#[async_trait]
impl Tool for RemoveAutomation {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "remove_automation".into(),
            description: "Delete an automation for good. Find it with list_automations, read \
                its sentence back and ask first ('Remove: when the office door opens, turn off \
                the office lights?'); call this only after a yes. To stop one for a while, \
                pause_automation instead."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let id = required("remove_automation", &args, "id")?;
        let gone = self
            .store
            .remove(id)
            .map_err(|e| invalid("remove_automation", e.to_string()))?;
        Ok(json!({ "removed": gone.id, "sentence": sentence(&gone) }))
    }
}

pub struct AutomationRuns {
    store: Arc<AutomationStore>,
    log: Arc<RunLog>,
}

#[async_trait]
impl Tool for AutomationRuns {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "automation_runs".into(),
            description: "What an automation did lately: 'why didn't the office light turn \
                off?', 'did the garden door rule run?'. Find its id with list_automations. Each \
                run says when, which trigger, and how it ended — done (with what it did), \
                stopped by a condition (and why), replaced by a newer run, or failed. Kept since \
                Niles last started."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let id = required("automation_runs", &args, "id")?;
        let rule = self
            .store
            .get(id)
            .ok_or_else(|| invalid("automation_runs", format!("there is no automation '{id}'")))?;
        let trigger = |run_trigger: &Option<String>| {
            run_trigger.as_ref().and_then(|t| {
                rule.triggers
                    .iter()
                    .position(|known| known.id.as_ref() == Some(t))
                    .map(|i| format!("trigger {}", i + 1))
            })
        };
        let runs: Vec<Value> = self
            .log
            .runs(id)
            .into_iter()
            .take(10)
            .map(|run| {
                json!({
                    "at": run.started_at.to_rfc3339(),
                    "trigger": trigger(&run.trigger),
                    "outcome": run.outcome,
                    "reason": run.reason,
                    "steps": run.steps,
                })
            })
            .collect();
        Ok(json!({ "sentence": sentence(&rule), "runs": runs }))
    }
}

fn required<'a>(tool: &str, args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(tool, format!("missing '{key}'")))
}

fn invalid(tool: &str, reason: String) -> Error {
    Error::InvalidArgs {
        tool: tool.into(),
        reason,
    }
}

/// Register the automation tools over the store the app and the engine
/// share, and the log the engine writes its runs to.
/// Only the list: for a background job that has to know what rules exist
/// — to tell a note about one from a fact — and may not change them.
pub fn register_automation_reading(reg: &mut ToolRegistry, store: Arc<AutomationStore>) {
    reg.register(Box::new(ListAutomations { store }));
}

pub fn register_automation_tools(
    reg: &mut ToolRegistry,
    store: Arc<AutomationStore>,
    registry: Arc<DeviceRegistry>,
    log: Arc<RunLog>,
    proposals: Arc<Proposals>,
    tz: chrono_tz::Tz,
) {
    reg.register(Box::new(ProposeAutomation {
        store: store.clone(),
        registry: registry.clone(),
        proposals: proposals.clone(),
        tz,
    }));
    reg.register(Box::new(CreateAutomation {
        store: store.clone(),
        registry,
        proposals,
    }));
    reg.register(Box::new(ListAutomations {
        store: store.clone(),
    }));
    reg.register(Box::new(PauseAutomation {
        store: store.clone(),
    }));
    reg.register(Box::new(RemoveAutomation {
        store: store.clone(),
    }));
    reg.register(Box::new(AutomationRuns { store, log }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{Device, DeviceClass, DeviceState};

    fn house() -> Arc<DeviceRegistry> {
        let registry = Arc::new(DeviceRegistry::new());
        let door = DeviceId::parse("z2m:office/door").unwrap();
        registry.upsert(Device::new(
            door,
            DeviceState {
                open: Some(false),
                ..Default::default()
            },
            DeviceClass::Contact,
        ));
        let go = DeviceId::parse("z2m:office/go").unwrap();
        registry.upsert(Device::new(
            go,
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        registry
    }

    fn garden_door(room: &str) -> Value {
        json!({
            "description": "Office lights off when the garden door opens",
            "triggers": [{ "type": "contact", "device": "office/door", "open": true }],
            "actions": [{ "do": "set_room", "room": room, "on": false }],
        })
    }

    #[tokio::test]
    async fn proposing_saves_nothing_and_says_the_rule() {
        let store = Arc::new(AutomationStore::new());
        let propose = ProposeAutomation {
            store: store.clone(),
            registry: house(),
            proposals: Arc::new(Proposals::new()),
            tz: chrono_tz::Tz::UTC,
        };
        let out = propose.execute(garden_door("office")).await.unwrap();
        assert_eq!(
            out["sentence"],
            "When the office door opens, turn off the office lights."
        );
        assert_eq!(out["problems"], json!([]));
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn a_misheard_room_is_a_problem_not_a_rule_even_inside_a_branch() {
        let store = Arc::new(AutomationStore::new());
        let propose = ProposeAutomation {
            store: store.clone(),
            registry: house(),
            proposals: Arc::new(Proposals::new()),
            tz: chrono_tz::Tz::UTC,
        };
        let out = propose.execute(garden_door("offers")).await.unwrap();
        assert_eq!(
            out["problems"],
            json!(["no lights in a room called offers"])
        );
        let branched = json!({
            "description": "x",
            "triggers": [{ "type": "contact", "device": "office/door", "open": false }],
            "actions": [{ "do": "if", "conditions": [], "then": [
                { "do": "delay", "minutes": 5 },
                { "do": "set_device", "device": "office/goo", "on": false }
            ] }],
        });
        let out = propose.execute(branched).await.unwrap();
        assert_eq!(out["problems"], json!(["no device called office/goo"]));
        let create = CreateAutomation {
            store: store.clone(),
            registry: house(),
            proposals: Arc::new(Proposals::new()),
        };
        assert!(create.execute(garden_door("offers")).await.is_err());
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn a_reading_the_device_does_not_report_is_a_problem() {
        let registry = house();
        let mut meter = Device::new(
            DeviceId::parse("z2m:system/electricity").unwrap(),
            DeviceState::default(),
            DeviceClass::Sensor,
        );
        meter.state.readings.insert(
            "price".into(),
            niles_core::Reading::new(1.2, Some("kr/kWh".into())),
        );
        // Declared but not yet reported: still a reading it has.
        meter
            .reading_units
            .insert("energy".into(), Some("kWh".into()));
        registry.upsert(meter);
        let propose = ProposeAutomation {
            store: Arc::new(AutomationStore::new()),
            registry,
            proposals: Arc::new(Proposals::new()),
            tz: chrono_tz::Tz::UTC,
        };
        let when = |measure: &str| {
            json!({
                "description": "x",
                "triggers": [{ "type": "numeric", "device": "system/electricity", "measure": measure, "below": 1 }],
                "actions": [{ "do": "if",
                    "conditions": [{ "kind": "numeric", "device": "office/door", "measure": "co2", "above": 1000 }],
                    "then": [{ "do": "set_room", "room": "office", "on": true }] }],
            })
        };
        let out = propose.execute(when("price")).await.unwrap();
        assert_eq!(
            out["problems"],
            json!(["office/door reports no reading called co2, nor any other"]),
            "found inside a branch too"
        );
        assert!(
            out["sentence"]
                .as_str()
                .unwrap()
                .starts_with("When the price of the system electricity goes below 1")
        );
        let out = propose.execute(when("energy")).await.unwrap();
        assert_eq!(out["problems"].as_array().unwrap().len(), 1);
        let out = propose.execute(when("powr")).await.unwrap();
        assert_eq!(
            out["problems"][1],
            "system/electricity reports no reading called powr; it reports energy, price"
        );
        // A fixed measure is not second-guessed.
        let out = propose.execute(when("temperature")).await.unwrap();
        assert_eq!(out["problems"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn made_paused_and_removed_by_voice() {
        let store = Arc::new(AutomationStore::new());
        let create = CreateAutomation {
            store: store.clone(),
            registry: house(),
            proposals: Arc::new(Proposals::new()),
        };
        let made = create.execute(garden_door("office")).await.unwrap();
        let id = made["id"].as_str().unwrap().to_string();
        assert!(store.get(&id).unwrap().enabled);

        let listed = ListAutomations {
            store: store.clone(),
        }
        .execute(json!({}))
        .await
        .unwrap();
        assert_eq!(listed["automations"][0]["paused"], false);

        PauseAutomation {
            store: store.clone(),
        }
        .execute(json!({ "id": id, "paused": true }))
        .await
        .unwrap();
        assert!(!store.get(&id).unwrap().enabled);

        RemoveAutomation {
            store: store.clone(),
        }
        .execute(json!({ "id": id }))
        .await
        .unwrap();
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn the_model_does_not_choose_the_id() {
        let store = Arc::new(AutomationStore::new());
        let create = CreateAutomation {
            store: store.clone(),
            registry: house(),
            proposals: Arc::new(Proposals::new()),
        };
        let mut rule = garden_door("office");
        rule["id"] = json!("whatever-it-likes");
        let made = create.execute(rule).await.unwrap();
        assert_eq!(made["id"], "office-lights-off-when-the-garden-door-opens");
    }

    #[tokio::test]
    async fn a_proposal_waits_for_its_answer() {
        let store = Arc::new(AutomationStore::new());
        let proposals = Arc::new(Proposals::new());
        let propose = ProposeAutomation {
            store: store.clone(),
            registry: house(),
            proposals: proposals.clone(),
            tz: chrono_tz::Tz::UTC,
        };
        propose.execute(garden_door("office")).await.unwrap();
        assert!(proposals.pending());
        let rule = proposals.take().unwrap();
        assert_eq!(
            rule.description,
            "Office lights off when the garden door opens"
        );
        assert!(!proposals.pending());
        // A misheard one is not offered.
        propose.execute(garden_door("offers")).await.unwrap();
        assert!(!proposals.pending());
    }

    #[tokio::test]
    async fn a_read_back_belongs_to_the_turn_it_was_made_in() {
        let proposals = Arc::new(Proposals::new());
        let propose = ProposeAutomation {
            store: Arc::new(AutomationStore::new()),
            registry: house(),
            proposals: proposals.clone(),
            tz: chrono_tz::Tz::UTC,
        };
        let before = Instant::now();
        propose.execute(garden_door("office")).await.unwrap();
        let after = Instant::now();
        let rule = proposals
            .offered_since(before)
            .expect("offered in this turn");
        assert!(proposals.offered_since(after).is_none());

        // Answered by a button, it no longer waits for a spoken yes.
        proposals.settled(&rule);
        assert!(!proposals.pending());
    }

    #[tokio::test]
    async fn a_time_written_as_a_condition_is_read_as_the_moment() {
        let store = Arc::new(
            AutomationStore::new()
                .with_location(niles_automations::sun::Location::of(56.1572, 10.2107)),
        );
        let propose = ProposeAutomation {
            store,
            registry: house(),
            proposals: Arc::new(Proposals::new()),
            tz: chrono_tz::Tz::UTC,
        };
        // As the model sent it, twice, on 2026-10-12.
        let out = propose
            .execute(json!({
                "description": "Close the curtains",
                "conditions": [{ "kind": "sun", "after": "sunset", "after_offset_minutes": 30 }],
                "actions": [{ "do": "notify", "body": "Please close the curtains." }],
            }))
            .await
            .unwrap();
        assert_eq!(out["problems"], json!([]));
        assert_eq!(
            out["sentence"],
            "When it is 30 minutes after sunset, say \"Please close the curtains.\"."
        );
    }

    #[test]
    fn knows_a_request_for_a_rule() {
        for said in [
            "make an automation that reminds me to close the curtains half an hour after the sun has set",
            "Niles, whenever the garden door opens, turn off the office light",
            "change the curtain rule to an hour after sunset",
            "remind me to take out the bins every Tuesday evening",
            "as soon as Mark comes home turn on the hallway",
            "wake me up seven hours after I go to bed",
        ] {
            assert!(about_automations(said), "{said}");
        }
        for said in [
            "when is my next meeting",
            "remind me in ten minutes",
            "turn off the office light",
            "what's the weather",
            "set a timer for 5 minutes",
            "wake me at six",
        ] {
            assert!(!about_automations(said), "{said}");
        }
    }

    #[test]
    fn a_rule_being_worked_out_holds_the_talk_until_answered() {
        let proposals = Proposals::new();
        assert!(!proposals.in_talk());
        proposals.discussed();
        assert!(proposals.in_talk());
        proposals.clear();
        assert!(!proposals.in_talk());
    }

    #[test]
    fn hears_a_plain_yes_or_no() {
        for yes in [
            "Yes",
            "yes please.",
            "Yeah",
            "Go ahead, Niles",
            "Okay",
            "Ja",
            "Yes, save it",
        ] {
            assert_eq!(answer(yes), Some(true), "{yes:?}");
        }
        for no in ["No", "No thanks", "Cancel", "never mind", "Nej"] {
            assert_eq!(answer(no), Some(false), "{no:?}");
        }
        for neither in [
            "Yes but at nine",
            "turn off the lights",
            "no wait, make it ten minutes",
        ] {
            assert_eq!(answer(neither), None, "{neither:?}");
        }
    }

    #[tokio::test]
    async fn says_why_a_rule_did_not_run() {
        let store = Arc::new(AutomationStore::new());
        let log = Arc::new(RunLog::new());
        let made = CreateAutomation {
            store: store.clone(),
            registry: house(),
            proposals: Arc::new(Proposals::new()),
        }
        .execute(garden_door("office"))
        .await
        .unwrap();
        let id = made["id"].as_str().unwrap().to_string();
        log.begin(
            &id,
            Some("t1"),
            chrono::Utc::now(),
            niles_automations::Outcome::Stopped,
            Some("it was not between 22:00 and 06:00".into()),
        );
        let out = AutomationRuns {
            store,
            log: log.clone(),
        }
        .execute(json!({ "id": id }))
        .await
        .unwrap();
        assert_eq!(out["runs"][0]["outcome"], "stopped");
        assert_eq!(out["runs"][0]["trigger"], "trigger 1");
        assert_eq!(
            out["runs"][0]["reason"],
            "it was not between 22:00 and 06:00"
        );
    }
}
