//! What a rule will not do the way it reads, said when it is saved or
//! read back.
//!
//! The one this knows: a run that waits — a trigger's `for`, or a
//! `delay` — and only then checks a sensor or a device before it acts.
//! If the check fails at that moment the run ends there, and unless one
//! of the rule's triggers follows that state into what the check wants,
//! nothing ever tries again. "Off once the hallway has been clear for
//! two minutes, if the kitchen is clear too" leaves the light on for
//! good if somebody was in the kitchen at that minute.
//!
//! Warnings, not refusals: the check may be meant to stop it. Kept
//! narrow on purpose, so the rule most motion lights are — on with
//! motion, off after a quiet spell — is never warned about.

use crate::rule::assign_trigger_ids;
use crate::sentence::{action, condition, trigger, trigger_names};
use crate::store::DEFAULT_SOURCE;
use niles_config::{ActionDto, AutomationRuleDto, ConditionDto, TriggerDto, TriggerKind};

/// What may not happen as the rule reads, each said so the person — or
/// the model that wrote it — can tell whether it is meant.
pub fn warnings(rule: &AutomationRuleDto) -> Vec<String> {
    let mut rule = rule.clone();
    assign_trigger_ids(&mut rule);
    let walk = Walk {
        triggers: &rule.triggers,
        names: trigger_names(&rule.triggers),
    };
    let everyone: Vec<usize> = (0..rule.triggers.len()).collect();
    let reach = walk.narrow(&everyone, &rule.conditions);
    let guards: Vec<Guard> = rule
        .conditions
        .iter()
        .filter_map(|c| Guard::of(c, false))
        .collect();
    let mut found = Vec::new();
    walk.actions(&rule.actions, &reach, &guards, false, &mut found);
    found.iter().map(|f| walk.say(f)).collect()
}

/// A check a run passes on its way to a step: a sensor or a device in
/// a state, and whether the run has waited before it gets there.
#[derive(Clone)]
struct Guard {
    condition: ConditionDto,
    late: bool,
}

impl Guard {
    fn of(c: &ConditionDto, late: bool) -> Option<Self> {
        matches!(
            c,
            ConditionDto::DeviceIs { .. }
                | ConditionDto::ContactIs { .. }
                | ConditionDto::MotionIs { .. }
        )
        .then(|| Guard {
            condition: c.clone(),
            late,
        })
    }
}

/// A run that may end at a check with nobody to try again: the trigger
/// that starts it, the check, and what it then does.
struct Stranded {
    trigger: usize,
    guard: Guard,
    steps: Vec<String>,
}

struct Walk<'a> {
    triggers: &'a [TriggerDto],
    names: Vec<(String, String)>,
}

impl Walk<'_> {
    /// The triggers in `reach` still there once `conditions` say which
    /// trigger started the run.
    fn narrow(&self, reach: &[usize], conditions: &[ConditionDto]) -> Vec<usize> {
        reach
            .iter()
            .copied()
            .filter(|&i| {
                conditions.iter().all(|c| match c {
                    ConditionDto::TriggeredBy { triggers } => {
                        triggers.iter().any(|id| *id == self.names[i].0)
                    }
                    _ => true,
                })
            })
            .collect()
    }

    fn actions(
        &self,
        actions: &[ActionDto],
        reach: &[usize],
        guards: &[Guard],
        mut late: bool,
        found: &mut Vec<Stranded>,
    ) {
        for a in actions {
            match a {
                ActionDto::Delay { .. } => late = true,
                ActionDto::If {
                    conditions,
                    then,
                    otherwise,
                } => {
                    // An `if` with an `else` chooses between two things
                    // to do rather than stopping, so only one without
                    // stops a run at its check.
                    let mut checked = guards.to_vec();
                    if otherwise.is_empty() {
                        checked.extend(conditions.iter().filter_map(|c| Guard::of(c, late)));
                    }
                    self.actions(then, &self.narrow(reach, conditions), &checked, late, found);
                    let reach: Vec<usize> = match conditions.as_slice() {
                        [ConditionDto::TriggeredBy { triggers }] => reach
                            .iter()
                            .copied()
                            .filter(|&i| !triggers.contains(&self.names[i].0))
                            .collect(),
                        _ => reach.to_vec(),
                    };
                    self.actions(otherwise, &reach, guards, late, found);
                }
                step => self.step(step, reach, guards, found),
            }
        }
    }

    fn step(&self, step: &ActionDto, reach: &[usize], guards: &[Guard], found: &mut Vec<Stranded>) {
        for guard in guards {
            if acts_on(step, &guard.condition) {
                continue;
            }
            let Some(&started) = reach
                .iter()
                .find(|&&i| guard.late || waits(&self.triggers[i].kind))
            else {
                continue;
            };
            if reach
                .iter()
                .any(|&i| follows(&self.triggers[i].kind, &guard.condition))
            {
                continue;
            }
            let done = action(step, &self.names);
            match found
                .iter_mut()
                .find(|f| f.trigger == started && f.guard.condition == guard.condition)
            {
                Some(f) if !f.steps.contains(&done) => f.steps.push(done),
                Some(_) => {}
                None => found.push(Stranded {
                    trigger: started,
                    guard: guard.clone(),
                    steps: vec![done],
                }),
            }
        }
    }

    fn say(&self, f: &Stranded) -> String {
        let started = &self.triggers[f.trigger];
        let check = condition(&f.guard.condition, &self.names);
        let steps = and_list(&f.steps);
        let mut out = format!(
            "A run started by {} ({}) checks that {check} only once its wait is over, before \
             it will {steps}. If that is not so at that moment the run ends, and nothing in this \
             rule tries again — so that may never happen.",
            self.names[f.trigger].1,
            trigger(started)
        );
        match &f.guard.condition {
            ConditionDto::MotionIs {
                detected: false, ..
            } => {
                let mut sensors = motion_sensors(&started.kind);
                for s in condition_sensors(&f.guard.condition) {
                    if !sensors.contains(&s) {
                        sensors.push(s);
                    }
                }
                let listed: Vec<String> = sensors.iter().map(|s| format!("{s:?}")).collect();
                out.push_str(&format!(
                    " To wait until none of them has seen anybody for a while, use one motion \
                     trigger over all the sensors — devices: [{}], detected: false, with for — \
                     instead of the condition.",
                    listed.join(", ")
                ));
            }
            _ => out.push_str(
                " If it should wait for that, add a trigger for it; if the condition is there \
                 to stop the run, this is as meant.",
            ),
        }
        out
    }
}

/// Whether a trigger waits before its run starts.
fn waits(t: &TriggerKind) -> bool {
    match t {
        TriggerKind::DeviceState { hold, .. }
        | TriggerKind::Contact { hold, .. }
        | TriggerKind::Motion { hold, .. }
        | TriggerKind::Numeric { hold, .. }
        | TriggerKind::Presence { hold, .. } => hold.is_some(),
        _ => false,
    }
}

/// Whether `t` fires when the state `check` wants comes about, so a run
/// stopped by it is tried again then.
fn follows(t: &TriggerKind, check: &ConditionDto) -> bool {
    match (t, check) {
        (
            TriggerKind::Motion { detected, .. },
            ConditionDto::MotionIs {
                detected: wanted, ..
            },
        ) => {
            detected.is_none_or(|d| d == *wanted)
                && overlap(&motion_sensors(t), &motion_rooms(t), check)
        }
        (
            TriggerKind::Contact { device, open, .. },
            ConditionDto::ContactIs {
                device: checked,
                open: wanted,
            },
        ) => same(device, checked) && open.is_none_or(|o| o == *wanted),
        (
            TriggerKind::DeviceState {
                device, room, on, ..
            },
            ConditionDto::DeviceIs {
                device: checked,
                on: wanted,
            },
        ) => {
            on.is_none_or(|o| o == *wanted)
                && match device {
                    Some(device) => same(device, checked),
                    None => room.as_deref().is_none_or(|r| r == room_of(checked)),
                }
        }
        _ => false,
    }
}

/// Whether a trigger's sensors and a condition's share one.
fn overlap(devices: &[String], rooms: &[String], check: &ConditionDto) -> bool {
    let checked = condition_sensors(check);
    let checked_rooms = match check {
        ConditionDto::MotionIs { rooms, .. } => rooms.clone(),
        _ => Vec::new(),
    };
    devices.iter().any(|d| {
        checked.iter().any(|c| same(d, c)) || checked_rooms.iter().any(|r| r == room_of(d))
    }) || rooms
        .iter()
        .any(|r| checked_rooms.contains(r) || checked.iter().any(|c| room_of(c) == r))
}

fn motion_sensors(t: &TriggerKind) -> Vec<String> {
    match t {
        TriggerKind::Motion {
            device, devices, ..
        } => device.iter().chain(devices).cloned().collect(),
        _ => Vec::new(),
    }
}

fn motion_rooms(t: &TriggerKind) -> Vec<String> {
    match t {
        TriggerKind::Motion { rooms, .. } => rooms.clone(),
        _ => Vec::new(),
    }
}

fn condition_sensors(c: &ConditionDto) -> Vec<String> {
    match c {
        ConditionDto::MotionIs {
            device, devices, ..
        } => device.iter().chain(devices).cloned().collect(),
        _ => Vec::new(),
    }
}

/// Whether a step changes the very device a check is about: "if the
/// hallway lamp is on, turn off the hallway lights" has nothing to try
/// again when the lamp is already off.
fn acts_on(step: &ActionDto, check: &ConditionDto) -> bool {
    let ConditionDto::DeviceIs {
        device: checked, ..
    } = check
    else {
        return false;
    };
    match step {
        ActionDto::SetDevice { device, .. } => same(device, checked),
        ActionDto::SetRoom { room, .. } => room == room_of(checked),
        ActionDto::SetBlinds { device, room, .. } => {
            device.as_deref().is_some_and(|d| same(d, checked))
                || room.as_deref().is_some_and(|r| r == room_of(checked))
        }
        _ => false,
    }
}

/// Two names for one device: `hallway/motion` is `z2m:hallway/motion`.
fn same(a: &str, b: &str) -> bool {
    let full = |d: &str| {
        if d.contains(':') {
            d.to_string()
        } else {
            format!("{DEFAULT_SOURCE}:{d}")
        }
    };
    full(a) == full(b)
}

fn room_of(device: &str) -> &str {
    let bare = device.split_once(':').map_or(device, |(_, rest)| rest);
    bare.split_once('/').map_or(bare, |(room, _)| room)
}

fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn warned(value: serde_json::Value) -> Vec<String> {
        warnings(&serde_json::from_value(value).unwrap())
    }

    fn motion(device: &str, detected: bool) -> serde_json::Value {
        json!({ "type": "motion", "device": device, "detected": detected })
    }

    fn quiet(device: &str) -> serde_json::Value {
        json!({ "type": "motion", "device": device, "detected": false, "for": { "minutes": 2 } })
    }

    fn lights(room: &str, on: bool) -> serde_json::Value {
        json!({ "do": "set_room", "room": room, "on": on })
    }

    fn started_by(id: &str, then: Vec<serde_json::Value>) -> serde_json::Value {
        json!({ "do": "if", "conditions": [{ "kind": "triggered_by", "triggers": [id] }], "then": then })
    }

    #[test]
    fn the_rule_most_motion_lights_are_is_not_warned_about() {
        let mut quiet_spell = quiet("hallway/motion");
        quiet_spell["id"] = json!("quiet");
        assert!(
            warned(json!({
                "id": "r",
                "triggers": [motion("hallway/motion", true), quiet_spell],
                "conditions": [{ "kind": "numeric", "device": "hallway/motion", "measure": "illuminance", "below": 30 }],
                "actions": [{
                    "do": "if",
                    "conditions": [{ "kind": "triggered_by", "triggers": ["quiet"] }],
                    "then": [lights("hallway", false)],
                    "else": [lights("hallway", true)],
                }],
            }))
            .is_empty()
        );
    }

    #[test]
    fn an_off_held_back_by_the_other_room_is_warned_about() {
        // As the model wrote it for the hallway and the kitchen.
        let warned = warned(json!({
            "id": "r",
            "triggers": [
                { "id": "hall-seen", "type": "motion", "device": "hallway/motion", "detected": true },
                { "id": "kitchen-seen", "type": "motion", "device": "kitchen/motion", "detected": true },
                { "id": "hall-quiet", "type": "motion", "device": "hallway/motion", "detected": false, "for": { "minutes": 2 } },
                { "id": "kitchen-quiet", "type": "motion", "device": "kitchen/motion", "detected": false, "for": { "minutes": 2 } }
            ],
            "actions": [
                started_by("hall-seen", vec![lights("hallway", true)]),
                started_by("kitchen-seen", vec![lights("kitchen", true), lights("hallway", true)]),
                {
                    "do": "if",
                    "conditions": [
                        { "kind": "triggered_by", "triggers": ["hall-quiet"] },
                        { "kind": "motion_is", "device": "kitchen/motion", "detected": false }
                    ],
                    "then": [lights("hallway", false)],
                },
                {
                    "do": "if",
                    "conditions": [
                        { "kind": "triggered_by", "triggers": ["kitchen-quiet"] },
                        { "kind": "motion_is", "device": "hallway/motion", "detected": false }
                    ],
                    "then": [lights("kitchen", false), lights("hallway", false)],
                }
            ],
        }));
        assert_eq!(warned.len(), 2, "{warned:#?}");
        assert!(
            warned[0].starts_with(
                "A run started by trigger 3 (the hallway motion has seen no motion for 2 minutes) \
                 checks that the kitchen motion sees no motion only once its wait is over, before \
                 it will turn off the hallway lights."
            ),
            "{}",
            warned[0]
        );
        assert!(
            warned[1].contains(
                "before it will turn off the kitchen lights and turn off the hallway lights"
            ),
            "{}",
            warned[1]
        );
        assert!(
            warned[1].contains(r#"devices: ["kitchen/motion", "hallway/motion"], detected: false"#),
            "{}",
            warned[1]
        );
    }

    #[test]
    fn a_check_at_once_is_meant_to_stop_it() {
        // "Hallway light on with motion, but not while the kitchen has
        // somebody in it": nothing waited, so nothing was missed.
        assert!(
            warned(json!({
                "id": "r",
                "triggers": [motion("hallway/motion", true)],
                "conditions": [{ "kind": "motion_is", "device": "kitchen/motion", "detected": false }],
                "actions": [lights("hallway", true)],
            }))
            .is_empty()
        );
    }

    #[test]
    fn a_wait_in_the_actions_counts_as_a_wait() {
        let warned = warned(json!({
            "id": "r",
            "triggers": [{ "type": "contact", "device": "office/door", "open": false }],
            "actions": [
                { "do": "delay", "minutes": 5 },
                { "do": "if",
                  "conditions": [{ "kind": "contact_is", "device": "office/window", "open": false }],
                  "then": [lights("office", false)] }
            ],
        }));
        assert_eq!(warned.len(), 1);
        assert!(
            warned[0].contains("checks that the office window is closed"),
            "{}",
            warned[0]
        );
        assert!(warned[0].contains("add a trigger for it"), "{}", warned[0]);
    }

    #[test]
    fn a_trigger_that_follows_the_checked_state_tries_again() {
        assert!(
            warned(json!({
                "id": "r",
                "triggers": [quiet("hallway/motion"), quiet("kitchen/motion")],
                "conditions": [{ "kind": "motion_is", "device": "kitchen/motion", "detected": false }],
                "actions": [lights("hallway", false)],
            }))
            .is_empty(),
            "the kitchen clearing starts it again"
        );
    }

    #[test]
    fn checking_the_light_it_turns_off_is_not_a_problem() {
        assert!(
            warned(json!({
                "id": "r",
                "triggers": [quiet("hallway/motion")],
                "conditions": [{ "kind": "device_is", "device": "hallway/ceiling", "on": true }],
                "actions": [lights("hallway", false)],
            }))
            .is_empty()
        );
    }

    #[test]
    fn one_trigger_over_both_rooms_is_not_warned_about() {
        assert!(
            warned(json!({
                "id": "r",
                "triggers": [
                    { "id": "hall-seen", "type": "motion", "device": "hallway/motion", "detected": true },
                    { "id": "kitchen-seen", "type": "motion", "device": "kitchen/motion", "detected": true },
                    { "id": "all-clear", "type": "motion", "devices": ["hallway/motion", "kitchen/motion"], "detected": false, "for": { "minutes": 2 } }
                ],
                "actions": [
                    started_by("hall-seen", vec![lights("hallway", true)]),
                    started_by("kitchen-seen", vec![lights("kitchen", true), lights("hallway", true)]),
                    started_by("all-clear", vec![lights("kitchen", false), lights("hallway", false)])
                ],
            }))
            .is_empty()
        );
    }

    #[test]
    fn a_choice_between_two_branches_is_not_a_run_stopped() {
        assert!(
            warned(json!({
                "id": "r",
                "triggers": [quiet("hallway/motion")],
                "actions": [{
                    "do": "if",
                    "conditions": [{ "kind": "motion_is", "device": "kitchen/motion", "detected": true }],
                    "then": [{ "do": "notify", "body": "Somebody is in the kitchen" }],
                    "else": [lights("hallway", false)],
                }],
            }))
            .is_empty()
        );
    }

    #[test]
    fn a_group_condition_after_a_wait_is_warned_about_with_its_sensors() {
        let warned = warned(json!({
            "id": "r",
            "triggers": [{ "type": "contact", "device": "office/door", "open": false, "for": { "minutes": 1 } }],
            "conditions": [{ "kind": "motion_is", "devices": ["office/motion", "hallway/motion"], "detected": false }],
            "actions": [lights("office", false)],
        }));
        assert_eq!(warned.len(), 1);
        assert!(
            warned[0].contains("checks that there is no motion in the office or the hallway"),
            "{}",
            warned[0]
        );
        assert!(
            warned[0].contains(r#"devices: ["office/motion", "hallway/motion"]"#),
            "{}",
            warned[0]
        );
    }
}
