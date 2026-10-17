//! Typed automation rule model: validation, what triggers follow,
//! condition evaluation, and how each part is said.

use crate::error::{Error, Result};
use crate::sun::{self, Location, SunEvent};
use chrono::{DateTime, Datelike, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use niles_config::{
    ActionDto, AutomationRuleDto, BlindsPositionDto, ConditionDto, DurationDto, TriggerKind,
};
use niles_core::{
    BlindsLevel, DeviceId, DeviceRegistry, DeviceState, Event, PresenceState, RoomName,
};
use std::time::Duration;

/// The most triggers a rule may have, and the longest it may wait.
pub const MAX_TRIGGERS: usize = 10;
pub const MAX_WAIT: Duration = Duration::from_secs(24 * 60 * 60);
/// How deep `if` may nest inside `if`.
pub const MAX_DEPTH: usize = 3;

/// A validated, ready-to-run automation rule.
#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub description: String,
    pub enabled: bool,
    pub triggers: Vec<RuleTrigger>,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
    /// What the rule was saved as, hashed: a wait kept over a restart
    /// carries on only if its rule is still this one.
    pub fingerprint: String,
}

/// One of a rule's triggers, with the id `triggered_by` knows it by
/// and how long its state must hold first, if at all.
#[derive(Debug, Clone)]
pub struct RuleTrigger {
    pub id: String,
    pub trigger: Trigger,
    pub hold: Option<Duration>,
}

/// What starts a rule.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Trigger {
    DeviceState {
        device: Option<DeviceId>,
        room: Option<RoomName>,
        on: Option<bool>,
    },
    /// A door or window sensor opening, closing, or either.
    Contact {
        device: DeviceId,
        open: Option<bool>,
    },
    /// A motion sensor seeing somebody, or no longer.
    Motion {
        device: DeviceId,
        detected: Option<bool>,
    },
    /// Several motion sensors as one: `detected: false` holds while none
    /// of them sees anybody, `true` while any of them does. With `for`,
    /// "no motion in the hallway or the kitchen for 2 minutes" — counted
    /// from the last of them to clear, and started over by motion in
    /// any. See [`MotionGroup`] for a sensor not heard from.
    MotionGroup {
        group: MotionGroup,
        detected: bool,
    },
    /// A reading coming to lie above and/or below a value.
    Numeric {
        device: DeviceId,
        measure: Measure,
        above: Option<f64>,
        below: Option<f64>,
    },
    DeviceAction {
        device: DeviceId,
        action: Option<String>,
    },
    TimerFired {
        name: Option<String>,
    },
    Presence {
        state: Option<PresenceFilter>,
    },
    Time {
        at: NaiveTime,
        days: Option<Vec<Weekday>>,
    },
    Sun {
        event: SunEvent,
        offset_minutes: i32,
    },
}

/// A number a numeric trigger or condition looks at: one of the fixed
/// fields, or a named reading.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Measure {
    Temperature,
    Humidity,
    Battery,
    Brightness,
    /// Light falling on a sensor, in lux.
    Illuminance,
    /// A named reading from `DeviceState::readings` — `price`, `power`,
    /// `co2`. Any valid name that is not one of the above, so a rule
    /// can follow a number nobody here has heard of.
    Reading(String),
}

impl Measure {
    /// `None` only for a name no reading could have. A misspelt reading
    /// is still a reading — whether the device has it is a question for
    /// whoever knows the device, which the rule tool asks.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        Some(match value.as_str() {
            "temperature" => Measure::Temperature,
            "humidity" => Measure::Humidity,
            "battery" => Measure::Battery,
            "brightness" => Measure::Brightness,
            "illuminance" | "lux" | "light_level" => Measure::Illuminance,
            _ => {
                niles_core::validate_reading_name(&value).ok()?;
                Measure::Reading(value)
            }
        })
    }

    pub fn read(&self, state: &DeviceState) -> Option<f64> {
        match self {
            Measure::Temperature => state.temperature_celsius.map(f64::from),
            Measure::Humidity => state.humidity_percent.map(f64::from),
            Measure::Battery => state.battery_percent.map(f64::from),
            Measure::Brightness => state.brightness.map(f64::from),
            Measure::Illuminance => state.illuminance_lux.map(f64::from),
            Measure::Reading(name) => state.readings.get(name).map(|r| r.value),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Measure::Temperature => "temperature",
            Measure::Humidity => "humidity",
            Measure::Battery => "battery",
            Measure::Brightness => "brightness",
            Measure::Illuminance => "illuminance",
            Measure::Reading(name) => name,
        }
    }
}

fn in_range(value: f64, above: Option<f64>, below: Option<f64>) -> bool {
    above.is_none_or(|a| value > a) && below.is_none_or(|b| value < b)
}

/// The motion sensors one trigger or condition watches together: some
/// by name, and every motion sensor in some rooms.
///
/// A room's sensors are looked up in the registry each time the group
/// is asked about, so one paired after the rule was saved belongs to it
/// from its first report.
///
/// A sensor counts only for what it is known to say. One that has never
/// reported motion, is not in the registry, or that its source says it
/// cannot reach, is not known to be clear: the group is not "no motion"
/// while it is in it — better a light left on than one switched off on
/// somebody. Nor does its last word count as motion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotionGroup {
    pub devices: Vec<DeviceId>,
    pub rooms: Vec<RoomName>,
}

/// What a group's state is filed under in the engine: one subject for
/// the whole group, whichever sensor spoke.
const GROUP_SUBJECT: &str = "motion sensors";

impl MotionGroup {
    /// Whether `id` is one of the group's sensors. A device in one of its
    /// rooms is, once it says anything about motion.
    fn includes(&self, id: &DeviceId) -> bool {
        self.devices.contains(id) || self.rooms.iter().any(|r| r == id.room())
    }

    /// What each sensor says: `Some(motion)` when it is known, `None`
    /// when it is not. `heard` is a report just in, which goes for its
    /// sensor whatever the registry has caught up with.
    fn readings(
        &self,
        registry: &DeviceRegistry,
        heard: Option<(&DeviceId, bool)>,
    ) -> Vec<Option<bool>> {
        let known = |device: &niles_core::Device| {
            if heard.is_some_and(|(id, _)| *id == device.id) {
                return heard.map(|(_, motion)| motion);
            }
            device.state.motion.filter(|_| device.available)
        };
        let mut seen: Vec<DeviceId> = Vec::new();
        let mut out = Vec::new();
        for id in &self.devices {
            seen.push(id.clone());
            out.push(match registry.get(id) {
                Some(device) => known(&device),
                None => heard.filter(|(h, _)| *h == id).map(|(_, m)| m),
            });
        }
        for room in &self.rooms {
            for device in registry.list_room(room) {
                let motion_sensor = device.class == niles_core::DeviceClass::Motion
                    || device.state.motion.is_some()
                    || heard.is_some_and(|(id, _)| *id == device.id);
                if motion_sensor && !seen.contains(&device.id) {
                    seen.push(device.id.clone());
                    out.push(known(&device));
                }
            }
        }
        out
    }

    /// Whether the group is as `detected` wants it: `false` every sensor
    /// known to see nobody (and at least one sensor), `true` any known
    /// to see somebody.
    fn holds(
        &self,
        registry: &DeviceRegistry,
        heard: Option<(&DeviceId, bool)>,
        detected: bool,
    ) -> bool {
        let readings = self.readings(registry, heard);
        if detected {
            readings.contains(&Some(true))
        } else {
            !readings.is_empty() && readings.iter().all(|r| *r == Some(false))
        }
    }

    /// "z2m:hallway/motion, z2m:kitchen/motion and every motion sensor
    /// in the office", for a run's reason.
    fn named(&self) -> String {
        let mut parts: Vec<String> = self.devices.iter().map(ToString::to_string).collect();
        parts.extend(
            self.rooms
                .iter()
                .map(|r| format!("every motion sensor in {r}")),
        );
        match parts.as_slice() {
            [] => String::new(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        }
    }
}

/// Gate that must pass after a trigger fires.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Condition {
    TimeOfDay {
        after: Option<NaiveTime>,
        before: Option<NaiveTime>,
        days: Option<Vec<Weekday>>,
    },
    Sun {
        after: Option<(SunEvent, i32)>,
        before: Option<(SunEvent, i32)>,
    },
    DeviceIs {
        device: DeviceId,
        on: bool,
    },
    ContactIs {
        device: DeviceId,
        open: bool,
    },
    MotionIs {
        device: DeviceId,
        detected: bool,
    },
    /// `detected: false` is none of the group seeing anybody, `true`
    /// any of them.
    MotionGroupIs {
        group: MotionGroup,
        detected: bool,
    },
    Numeric {
        device: DeviceId,
        measure: Measure,
        above: Option<f64>,
        below: Option<f64>,
    },
    PresenceIs(PresenceFilter),
    TriggeredBy(Vec<String>),
}

/// What to do when the rule fires.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Action {
    SetDevice {
        device: DeviceId,
        on: Option<bool>,
        brightness: Option<u8>,
        kelvin: Option<u16>,
    },
    /// Every light in the room.
    SetRoom {
        room: RoomName,
        on: Option<bool>,
        brightness: Option<u8>,
        kelvin: Option<u16>,
    },
    /// One blind, or every blind in a room.
    SetBlinds {
        target: BlindTarget,
        level: BlindsLevel,
    },
    Notify {
        body: String,
        room: Option<String>,
        priority: Priority,
    },
    /// The morning routine, now.
    WakeUp(WakePlan),
    Delay(Duration),
    If {
        conditions: Vec<Condition>,
        then: Vec<Action>,
        otherwise: Vec<Action>,
    },
}

/// A wake-up's settings, as a rule gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakePlan {
    /// How long the ramp takes.
    pub minutes: u32,
    pub rooms: Vec<RoomName>,
    pub devices: Vec<DeviceId>,
    pub except: Vec<DeviceId>,
    /// Where the ramp ends.
    pub brightness: u8,
    /// The warmth it starts at; `kelvin` when `None`. Only ever set
    /// with `kelvin`.
    pub start_kelvin: Option<u16>,
    /// The warmth it ends at. `None` follows the curve's as it goes, or
    /// leaves the colour be with the curve off.
    pub kelvin: Option<u16>,
}

/// A wake-up's ramp, unless the rule says otherwise.
pub const WAKE_UP_MINUTES: u32 = 45;
/// The longest a wake-up's ramp may take.
pub const MAX_WAKE_UP_MINUTES: u32 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlindTarget {
    Device(DeviceId),
    Room(RoomName),
}

/// Optional filter for the `Presence` trigger and condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresenceFilter {
    Home,
    Away,
    Unknown,
}

/// Local priority enum — kept inside `niles-automations` so the crate
/// does not depend on `niles-notifications`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Priority {
    Routine,
    Important,
    Urgent,
}

/// What a condition is evaluated against.
pub struct Context<'a> {
    pub registry: &'a DeviceRegistry,
    pub now: DateTime<Utc>,
    pub tz: Tz,
    pub location: Option<Location>,
    pub presence: Option<PresenceState>,
    /// The id of the trigger that started this run.
    pub trigger: Option<&'a str>,
}

/// Give every trigger an id: the one it has, or the first free of
/// `t1`, `t2`… — what `triggered_by` refers to, and what is stored.
pub fn assign_trigger_ids(dto: &mut AutomationRuleDto) {
    for i in 0..dto.triggers.len() {
        if dto.triggers[i]
            .id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
        {
            continue;
        }
        let taken = |candidate: &str| {
            dto.triggers
                .iter()
                .any(|t| t.id.as_deref() == Some(candidate))
        };
        let id = (1..)
            .map(|n| format!("t{n}"))
            .find(|c| !taken(c))
            .expect("an unbounded range always has a free name");
        dto.triggers[i].id = Some(id);
    }
}

impl Rule {
    /// Convert a DTO into a typed, validated `Rule`. Triggers without an
    /// id are given one, as [`assign_trigger_ids`] would.
    pub fn from_dto(dto: &AutomationRuleDto, default_source: &str) -> Result<Self> {
        let mut dto = dto.clone();
        assign_trigger_ids(&mut dto);
        let id = dto.id.clone();
        validate_id(&id)?;
        let invalid = |reason: String| Error::Invalid {
            id: id.clone(),
            reason,
        };

        if dto.triggers.is_empty() {
            return Err(invalid(
                "a rule needs a trigger: the moment it starts, in `triggers` — \
                 e.g. {\"type\": \"sun\", \"event\": \"sunset\", \"offset_minutes\": 30}"
                    .into(),
            ));
        }
        if dto.triggers.len() > MAX_TRIGGERS {
            return Err(invalid(format!("at most {MAX_TRIGGERS} triggers")));
        }
        let mut triggers = Vec::with_capacity(dto.triggers.len());
        for t in &dto.triggers {
            let trigger_id = t.id.clone().unwrap_or_default();
            if triggers
                .iter()
                .any(|known: &RuleTrigger| known.id == trigger_id)
            {
                return Err(invalid(format!("two triggers are called {trigger_id:?}")));
            }
            let (trigger, hold) = parse_trigger(&id, &t.kind, default_source)?;
            triggers.push(RuleTrigger {
                id: trigger_id,
                trigger,
                hold,
            });
        }
        let ids: Vec<&str> = triggers.iter().map(|t| t.id.as_str()).collect();
        let conditions = dto
            .conditions
            .iter()
            .map(|c| parse_condition(&id, c, default_source, &ids))
            .collect::<Result<Vec<_>>>()?;
        let actions = parse_actions(&id, &dto.actions, default_source, &ids, 0)?;
        if actions.is_empty() {
            return Err(Error::NoActions { id });
        }

        let fingerprint = {
            use sha2::Digest;
            let saved = serde_json::to_vec(&dto).unwrap_or_default();
            hex::encode(sha2::Sha256::digest(&saved))
        };
        Ok(Self {
            id,
            description: dto.description.clone(),
            enabled: dto.enabled,
            triggers,
            conditions,
            actions,
            fingerprint,
        })
    }

    /// Whether a rule started only by the clock could never get past
    /// its own time and sun conditions: "at sunset, if it is 30 minutes
    /// after sunset" is never true at sunset. Checked for the day of
    /// `now`; a rule with any other kind of trigger is not judged.
    pub fn never_runs(
        &self,
        registry: &DeviceRegistry,
        now: DateTime<Utc>,
        tz: Tz,
        location: Option<Location>,
    ) -> bool {
        let timed: Vec<&Condition> = self
            .conditions
            .iter()
            .filter(|c| matches!(c, Condition::TimeOfDay { .. } | Condition::Sun { .. }))
            .collect();
        if timed.is_empty() {
            return false;
        }
        let mut due = Vec::with_capacity(self.triggers.len());
        for t in &self.triggers {
            match t.trigger.due_on(now, tz, location) {
                Some(at) => due.push((t.id.as_str(), at)),
                None => return false,
            }
        }
        !due.iter().any(|(id, at)| {
            let Some(at) = tz.from_local_datetime(at).earliest() else {
                return true;
            };
            let ctx = Context {
                registry,
                now: at.with_timezone(&Utc),
                tz,
                location,
                presence: None,
                trigger: Some(id),
            };
            timed.iter().all(|c| c.evaluate(&ctx))
        })
    }

    /// Whether anything in it needs to know where the sun is.
    pub fn uses_sun(&self) -> bool {
        fn in_conditions(c: &[Condition]) -> bool {
            c.iter().any(|c| matches!(c, Condition::Sun { .. }))
        }
        fn in_actions(a: &[Action]) -> bool {
            a.iter().any(|a| match a {
                Action::If {
                    conditions,
                    then,
                    otherwise,
                } => in_conditions(conditions) || in_actions(then) || in_actions(otherwise),
                _ => false,
            })
        }
        self.triggers
            .iter()
            .any(|t| matches!(t.trigger, Trigger::Sun { .. }))
            || in_conditions(&self.conditions)
            || in_actions(&self.actions)
    }
}

impl Trigger {
    /// Whether it follows a state — fires on the change into it, and
    /// can be asked to have held for a while — rather than a moment.
    pub fn follows_state(&self) -> bool {
        matches!(
            self,
            Trigger::DeviceState { .. }
                | Trigger::Contact { .. }
                | Trigger::Motion { .. }
                | Trigger::MotionGroup { .. }
                | Trigger::Numeric { .. }
                | Trigger::Presence { .. }
        )
    }

    /// For a trigger that follows a state: what `ev` says of it, as the
    /// thing it concerns (a device, a group of sensors, or the house)
    /// and whether the state now holds. `None` when the event says
    /// nothing about it. A group is read across all its sensors, the
    /// others as `registry` has them.
    pub fn observe(&self, ev: &Event, registry: &DeviceRegistry) -> Option<(String, bool)> {
        match (self, ev) {
            (Trigger::MotionGroup { group, detected }, Event::DeviceStateChanged { id, state })
                if group.includes(id) =>
            {
                let now = state.motion?;
                Some((
                    GROUP_SUBJECT.into(),
                    group.holds(registry, Some((id, now)), *detected),
                ))
            }
            (
                Trigger::DeviceState { device, room, on },
                Event::DeviceStateChanged { id, state },
            ) => {
                if device.as_ref().is_some_and(|d| d != id)
                    || room.as_ref().is_some_and(|r| r != id.room())
                {
                    return None;
                }
                let now = state.on?;
                Some((id.to_string(), on.is_none_or(|want| want == now)))
            }
            (Trigger::Contact { device, open }, Event::DeviceStateChanged { id, state })
                if id == device =>
            {
                let now = state.open?;
                Some((id.to_string(), open.is_none_or(|want| want == now)))
            }
            (Trigger::Motion { device, detected }, Event::DeviceStateChanged { id, state })
                if id == device =>
            {
                let now = state.motion?;
                Some((id.to_string(), detected.is_none_or(|want| want == now)))
            }
            (
                Trigger::Numeric {
                    device,
                    measure,
                    above,
                    below,
                },
                Event::DeviceStateChanged { id, state },
            ) if id == device => {
                let value = measure.read(state)?;
                Some((id.to_string(), in_range(value, *above, *below)))
            }
            (Trigger::Presence { state }, Event::PresenceChanged { state: now }) => Some((
                "presence".into(),
                state.is_none_or(|want| presence_filter_matches(want, *now)),
            )),
            _ => None,
        }
    }

    /// For a trigger that follows a state: whether it holds now, by the
    /// registry, for each thing it concerns — what a start compares the
    /// first change with.
    pub fn seed(&self, registry: &DeviceRegistry) -> Vec<(String, bool)> {
        let event_of = |device: &niles_core::Device| Event::DeviceStateChanged {
            id: device.id.clone(),
            state: device.state.clone(),
        };
        match self {
            Trigger::Contact { device, .. }
            | Trigger::Motion { device, .. }
            | Trigger::Numeric { device, .. } => registry
                .get(device)
                .and_then(|d| self.observe(&event_of(&d), registry))
                .into_iter()
                .collect(),
            Trigger::DeviceState { .. } => registry
                .list_all()
                .iter()
                .filter_map(|d| self.observe(&event_of(d), registry))
                .collect(),
            // Always, known or not: a group none of whose sensors has
            // said anything yet is not clear, and its first all-clear is
            // a change.
            Trigger::MotionGroup { group, detected } => {
                vec![(GROUP_SUBJECT.into(), group.holds(registry, None, *detected))]
            }
            _ => Vec::new(),
        }
    }

    /// For a trigger that is a moment — a button, a timer — whether
    /// `ev` is that moment.
    pub fn fires(&self, ev: &Event) -> bool {
        match (self, ev) {
            (
                Trigger::DeviceAction { device, action },
                Event::DeviceAction {
                    id,
                    action: pressed,
                },
            ) => id == device && action.as_deref().is_none_or(|a| a == pressed),
            (Trigger::TimerFired { name }, Event::TimerFired { name: fired, .. }) => {
                name.is_none() || name.as_ref() == fired.as_ref()
            }
            _ => false,
        }
    }

    /// Whether `ev` matches it at all: a moment it is, or a state it
    /// follows coming to hold.
    pub fn matches(&self, ev: &Event, registry: &DeviceRegistry) -> bool {
        self.fires(ev) || self.observe(ev, registry).is_some_and(|(_, holds)| holds)
    }

    /// For a clock trigger: the minute it falls on, on the local date of
    /// `now`, as a local wall-clock time.
    pub fn due_on(
        &self,
        now: DateTime<Utc>,
        tz: Tz,
        location: Option<Location>,
    ) -> Option<NaiveDateTime> {
        let local = now.with_timezone(&tz);
        let day = local.date_naive();
        match self {
            Trigger::Time { at, days } => days
                .as_ref()
                .is_none_or(|d| d.contains(&local.weekday()))
                .then(|| day.and_time(*at)),
            Trigger::Sun {
                event,
                offset_minutes,
            } => sun::at(*event, *offset_minutes, day, location?, tz)?
                .naive_local()
                .with_second(0)?
                .with_nanosecond(0),
            _ => None,
        }
    }
}

impl Condition {
    /// Evaluate this condition against the world as `ctx` has it.
    pub fn evaluate(&self, ctx: &Context<'_>) -> bool {
        match self {
            Condition::TimeOfDay {
                after,
                before,
                days,
            } => {
                let local = ctx.now.with_timezone(&ctx.tz);
                days.as_ref().is_none_or(|d| d.contains(&local.weekday()))
                    && covers(*after, *before, local.time())
            }
            Condition::Sun { after, before } => {
                let Some(location) = ctx.location else {
                    return false;
                };
                let local = ctx.now.with_timezone(&ctx.tz);
                let day = local.date_naive();
                let bound = |b: Option<(SunEvent, i32)>| -> Option<Option<NaiveTime>> {
                    match b {
                        None => Some(None),
                        Some((event, offset)) => {
                            sun::at(event, offset, day, location, ctx.tz).map(|t| Some(t.time()))
                        }
                    }
                };
                match (bound(*after), bound(*before)) {
                    (Some(a), Some(b)) => covers(a, b, local.time()),
                    _ => false,
                }
            }
            Condition::DeviceIs { device, on } => ctx
                .registry
                .get(device)
                .and_then(|d| d.state.on)
                .is_some_and(|s| s == *on),
            Condition::ContactIs { device, open } => ctx
                .registry
                .get(device)
                .and_then(|d| d.state.open)
                .is_some_and(|s| s == *open),
            Condition::MotionIs { device, detected } => ctx
                .registry
                .get(device)
                .and_then(|d| d.state.motion)
                .is_some_and(|s| s == *detected),
            Condition::MotionGroupIs { group, detected } => {
                group.holds(ctx.registry, None, *detected)
            }
            Condition::Numeric {
                device,
                measure,
                above,
                below,
            } => ctx
                .registry
                .get(device)
                .and_then(|d| measure.read(&d.state))
                .is_some_and(|v| in_range(v, *above, *below)),
            Condition::PresenceIs(want) => ctx
                .presence
                .is_some_and(|now| presence_filter_matches(*want, now)),
            Condition::TriggeredBy(ids) => {
                ctx.trigger.is_some_and(|t| ids.iter().any(|id| id == t))
            }
        }
    }

    /// Why a run stopped here, in a few words.
    pub fn unmet(&self) -> String {
        match self {
            Condition::TimeOfDay { after, before, .. } => match (after, before) {
                (Some(a), Some(b)) => format!(
                    "it was not between {} and {}",
                    a.format("%H:%M"),
                    b.format("%H:%M")
                ),
                (Some(a), None) => format!("it was before {}", a.format("%H:%M")),
                (None, Some(b)) => format!("it was after {}", b.format("%H:%M")),
                (None, None) => "it was not one of the days".into(),
            },
            Condition::Sun { .. } => "the sun was not where the rule wants it".into(),
            Condition::DeviceIs { device, on } => {
                format!("{device} was not {}", if *on { "on" } else { "off" })
            }
            Condition::ContactIs { device, open } => {
                format!("{device} was not {}", if *open { "open" } else { "closed" })
            }
            Condition::MotionIs { device, detected } => {
                if *detected {
                    format!("{device} saw no motion")
                } else {
                    format!("{device} saw motion")
                }
            }
            Condition::MotionGroupIs { group, detected } => {
                if *detected {
                    format!("none of {} saw motion", group.named())
                } else {
                    format!(
                        "one of {} saw motion, or had not been heard from",
                        group.named()
                    )
                }
            }
            Condition::Numeric {
                device, measure, ..
            } => format!("the {} of {device} was out of range", measure.name()),
            Condition::PresenceIs(want) => format!("the house was not {want:?}").to_lowercase(),
            Condition::TriggeredBy(_) => "another trigger started it".into(),
        }
    }
}

impl Action {
    /// What a step did, for a run's history.
    pub fn done(&self) -> String {
        let how = |on: &Option<bool>, brightness: &Option<u8>, kelvin: &Option<u16>| {
            let mut parts = Vec::new();
            match on {
                Some(true) => parts.push("on".to_string()),
                Some(false) => parts.push("off".to_string()),
                None => {}
            }
            if let Some(b) = brightness {
                parts.push(format!("{b}%"));
            }
            if let Some(k) = kelvin {
                parts.push(format!("{k} K"));
            }
            parts.join(", ")
        };
        match self {
            Action::SetDevice {
                device,
                on,
                brightness,
                kelvin,
            } => format!("set {device} {}", how(on, brightness, kelvin)),
            Action::SetRoom {
                room,
                on,
                brightness,
                kelvin,
            } => format!("set the {room} lights {}", how(on, brightness, kelvin)),
            Action::SetBlinds { target, level } => {
                let what = match target {
                    BlindTarget::Device(device) => device.to_string(),
                    BlindTarget::Room(room) => format!("the {room} blinds"),
                };
                match level {
                    BlindsLevel::Exact(p) => format!("set {what} to {p}% open"),
                    BlindsLevel::Open => format!("opened {what}"),
                }
            }
            Action::Notify { body, room, .. } => match room {
                Some(room) => format!("said \"{body}\" in {room}"),
                None => format!("said \"{body}\""),
            },
            Action::WakeUp(plan) => format!(
                "started a {}-minute wake-up to {}%",
                plan.minutes, plan.brightness
            ),
            Action::Delay(wait) => format!("waited {}", say_duration(*wait)),
            Action::If { .. } => "checked a condition".into(),
        }
    }
}

/// "5 minutes", "1 hour 30 minutes", "45 seconds".
pub fn say_duration(wait: Duration) -> String {
    let total = wait.as_secs();
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    let unit = |n: u64, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(unit(h, "hour"));
    }
    if m > 0 {
        parts.push(unit(m, "minute"));
    }
    if s > 0 || parts.is_empty() {
        parts.push(unit(s, "second"));
    }
    parts.join(" ")
}

// ------------------------------------------------------------------
// Parsing
// ------------------------------------------------------------------

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::InvalidId {
            id: id.to_string(),
            reason: "empty".into(),
        });
    }
    if id.starts_with('-') || id.ends_with('-') {
        return Err(Error::InvalidId {
            id: id.to_string(),
            reason: "leading or trailing hyphen".into(),
        });
    }
    if id.contains("--") {
        return Err(Error::InvalidId {
            id: id.to_string(),
            reason: "double hyphen".into(),
        });
    }
    if let Some(c) = id
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
    {
        return Err(Error::InvalidId {
            id: id.to_string(),
            reason: format!("'{c}' not in [a-z0-9-]"),
        });
    }
    Ok(())
}

fn wait_of(id: &str, what: &str, d: &DurationDto) -> Result<Duration> {
    let wait = Duration::from_secs(d.total_seconds());
    if wait.is_zero() || wait > MAX_WAIT {
        return Err(Error::Invalid {
            id: id.to_string(),
            reason: format!("{what} must be more than nothing and at most 24 hours"),
        });
    }
    Ok(wait)
}

fn range(id: &str, above: Option<f64>, below: Option<f64>) -> Result<()> {
    match (above, below) {
        (None, None) => Err(Error::Invalid {
            id: id.to_string(),
            reason: "a reading needs a value to be above or below".into(),
        }),
        (Some(a), Some(b)) if a >= b => Err(Error::Invalid {
            id: id.to_string(),
            reason: format!("above {a} and below {b} can never both hold"),
        }),
        _ => Ok(()),
    }
}

fn parse_measure(id: &str, value: &str) -> Result<Measure> {
    Measure::parse(value).ok_or_else(|| Error::Invalid {
        id: id.to_string(),
        reason: format!(
            "{value:?} is not a measure; use temperature, humidity, battery, brightness, \
             illuminance, or a reading name made of a-z, 0-9 and _ (power, co2, price)"
        ),
    })
}

fn parse_sun(id: &str, value: &str) -> Result<SunEvent> {
    SunEvent::parse(value).ok_or_else(|| Error::Invalid {
        id: id.to_string(),
        reason: format!("{value:?} is not sunrise or sunset"),
    })
}

fn parse_days(id: &str, days: Option<&Vec<String>>) -> Result<Option<Vec<Weekday>>> {
    let Some(days) = days else { return Ok(None) };
    days.iter()
        .map(|d| {
            d.trim().parse::<Weekday>().map_err(|_| Error::Invalid {
                id: id.to_string(),
                reason: format!("{d:?} is not a weekday"),
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(|d| (!d.is_empty()).then_some(d))
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per trigger kind; splitting it scatters the validation table"
)]
fn parse_trigger(
    id: &str,
    dto: &TriggerKind,
    default_source: &str,
) -> Result<(Trigger, Option<Duration>)> {
    let hold = |h: &Option<DurationDto>| -> Result<Option<Duration>> {
        h.as_ref().map(|d| wait_of(id, "for", d)).transpose()
    };
    Ok(match dto {
        TriggerKind::DeviceState {
            device,
            room,
            on,
            hold: h,
        } => (
            Trigger::DeviceState {
                device: device
                    .as_ref()
                    .map(|s| parse_device_id(id, s, default_source))
                    .transpose()?,
                room: room.as_ref().map(|s| parse_room_name(id, s)).transpose()?,
                on: *on,
            },
            hold(h)?,
        ),
        TriggerKind::Contact {
            device,
            open,
            hold: h,
        } => (
            Trigger::Contact {
                device: parse_device_id(id, device, default_source)?,
                open: *open,
            },
            hold(h)?,
        ),
        TriggerKind::Motion {
            device,
            devices,
            rooms,
            detected,
            hold: h,
        } => {
            let trigger = match parse_sensors(
                id,
                "motion trigger",
                device.as_deref(),
                devices,
                rooms,
                default_source,
            )? {
                Sensors::One(device) => Trigger::Motion {
                    device,
                    detected: *detected,
                },
                Sensors::Group(group) => Trigger::MotionGroup {
                    group,
                    detected: detected.ok_or_else(|| Error::Invalid {
                        id: id.to_string(),
                        reason: "a motion trigger over several sensors needs `detected`: true \
                                 starts it when any of them sees somebody, false once none of \
                                 them does — with `for`, once none of them has for that long"
                            .into(),
                    })?,
                },
            };
            (trigger, hold(h)?)
        }
        TriggerKind::Numeric {
            device,
            measure,
            above,
            below,
            hold: h,
        } => {
            range(id, *above, *below)?;
            (
                Trigger::Numeric {
                    device: parse_device_id(id, device, default_source)?,
                    measure: parse_measure(id, measure)?,
                    above: *above,
                    below: *below,
                },
                hold(h)?,
            )
        }
        TriggerKind::DeviceAction { device, action } => (
            Trigger::DeviceAction {
                device: parse_device_id(id, device, default_source)?,
                action: action.clone(),
            },
            None,
        ),
        TriggerKind::TimerFired { name } => (Trigger::TimerFired { name: name.clone() }, None),
        TriggerKind::Presence { state, hold: h } => (
            Trigger::Presence {
                state: parse_presence_filter(id, state.as_deref())?,
            },
            hold(h)?,
        ),
        TriggerKind::Time { at, days } => (
            Trigger::Time {
                at: parse_time(id, at)?,
                days: parse_days(id, days.as_ref())?,
            },
            None,
        ),
        TriggerKind::Sun {
            event,
            offset_minutes,
        } => (
            Trigger::Sun {
                event: parse_sun(id, event)?,
                offset_minutes: offset_minutes.unwrap_or(0),
            },
            None,
        ),
    })
}

fn parse_condition(
    id: &str,
    dto: &ConditionDto,
    default_source: &str,
    triggers: &[&str],
) -> Result<Condition> {
    Ok(match dto {
        ConditionDto::TimeOfDay {
            after,
            before,
            days,
        } => Condition::TimeOfDay {
            after: after.as_ref().map(|s| parse_time(id, s)).transpose()?,
            before: before.as_ref().map(|s| parse_time(id, s)).transpose()?,
            days: parse_days(id, days.as_ref())?,
        },
        ConditionDto::Sun {
            after,
            before,
            after_offset_minutes,
            before_offset_minutes,
        } => {
            if after.is_none() && before.is_none() {
                return Err(Error::Invalid {
                    id: id.to_string(),
                    reason: "a sun condition needs an after or a before".into(),
                });
            }
            Condition::Sun {
                after: after
                    .as_ref()
                    .map(|e| parse_sun(id, e).map(|e| (e, after_offset_minutes.unwrap_or(0))))
                    .transpose()?,
                before: before
                    .as_ref()
                    .map(|e| parse_sun(id, e).map(|e| (e, before_offset_minutes.unwrap_or(0))))
                    .transpose()?,
            }
        }
        ConditionDto::DeviceIs { device, on } => Condition::DeviceIs {
            device: parse_device_id(id, device, default_source)?,
            on: *on,
        },
        ConditionDto::ContactIs { device, open } => Condition::ContactIs {
            device: parse_device_id(id, device, default_source)?,
            open: *open,
        },
        ConditionDto::MotionIs {
            device,
            devices,
            rooms,
            detected,
        } => match parse_sensors(
            id,
            "motion condition",
            device.as_deref(),
            devices,
            rooms,
            default_source,
        )? {
            Sensors::One(device) => Condition::MotionIs {
                device,
                detected: *detected,
            },
            Sensors::Group(group) => Condition::MotionGroupIs {
                group,
                detected: *detected,
            },
        },
        ConditionDto::Numeric {
            device,
            measure,
            above,
            below,
        } => {
            range(id, *above, *below)?;
            Condition::Numeric {
                device: parse_device_id(id, device, default_source)?,
                measure: parse_measure(id, measure)?,
                above: *above,
                below: *below,
            }
        }
        ConditionDto::PresenceIs { state } => Condition::PresenceIs(
            parse_presence_filter(id, Some(state))?.unwrap_or(PresenceFilter::Home),
        ),
        ConditionDto::TriggeredBy { triggers: named } => {
            if named.is_empty() {
                return Err(Error::Invalid {
                    id: id.to_string(),
                    reason: "\"triggered by\" names no trigger".into(),
                });
            }
            if let Some(gone) = named.iter().find(|n| !triggers.contains(&n.as_str())) {
                return Err(Error::Invalid {
                    id: id.to_string(),
                    reason: format!(
                        "\"triggered by\" names {gone:?}, which the rule does not have"
                    ),
                });
            }
            Condition::TriggeredBy(named.clone())
        }
    })
}

/// The sensors a motion trigger or condition names.
enum Sensors {
    One(DeviceId),
    Group(MotionGroup),
}

/// `device`, or `devices` and/or `rooms` — never both ways at once. A
/// list of one is that one sensor, and a name given twice counts once.
fn parse_sensors(
    id: &str,
    what: &str,
    device: Option<&str>,
    devices: &[String],
    rooms: &[String],
    default_source: &str,
) -> Result<Sensors> {
    let invalid = |reason: String| Error::Invalid {
        id: id.to_string(),
        reason,
    };
    let several = !devices.is_empty() || !rooms.is_empty();
    match device {
        Some(_) if several => Err(invalid(format!(
            "a {what} names one sensor in `device`, or several in `devices` (or every motion \
             sensor in `rooms`) — not both"
        ))),
        Some(device) => Ok(Sensors::One(parse_device_id(id, device, default_source)?)),
        None if !several => Err(invalid(format!(
            "a {what} needs a sensor: one in `device`, several in `devices`, or every motion \
             sensor in `rooms`"
        ))),
        None => {
            let mut named: Vec<DeviceId> = Vec::new();
            for d in devices {
                let d = parse_device_id(id, d, default_source)?;
                if !named.contains(&d) {
                    named.push(d);
                }
            }
            let mut places: Vec<RoomName> = Vec::new();
            for r in rooms {
                let r = parse_room_name(id, r)?;
                if !places.contains(&r) {
                    places.push(r);
                }
            }
            if let ([one], []) = (named.as_slice(), places.as_slice()) {
                return Ok(Sensors::One(one.clone()));
            }
            Ok(Sensors::Group(MotionGroup {
                devices: named,
                rooms: places,
            }))
        }
    }
}

fn parse_actions(
    id: &str,
    dtos: &[ActionDto],
    default_source: &str,
    triggers: &[&str],
    depth: usize,
) -> Result<Vec<Action>> {
    dtos.iter()
        .map(|a| parse_action(id, a, default_source, triggers, depth))
        .collect()
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per action kind; splitting it scatters the validation table"
)]
fn parse_action(
    id: &str,
    dto: &ActionDto,
    default_source: &str,
    triggers: &[&str],
    depth: usize,
) -> Result<Action> {
    Ok(match dto {
        ActionDto::SetDevice {
            device,
            on,
            brightness,
            kelvin,
        } => {
            check_light(id, *brightness, *kelvin)?;
            Action::SetDevice {
                device: parse_device_id(id, device, default_source)?,
                on: *on,
                brightness: *brightness,
                kelvin: *kelvin,
            }
        }
        ActionDto::SetRoom {
            room,
            on,
            brightness,
            kelvin,
        } => {
            check_light(id, *brightness, *kelvin)?;
            Action::SetRoom {
                room: parse_room_name(id, room)?,
                on: *on,
                brightness: *brightness,
                kelvin: *kelvin,
            }
        }
        ActionDto::SetBlinds {
            device,
            room,
            position,
        } => {
            let level = match position {
                BlindsPositionDto::Open => BlindsLevel::Open,
                BlindsPositionDto::Exact(p) => {
                    // The DTO can also be built in Rust, where the
                    // deserializer never ran — keep the range check.
                    if *p > 100 {
                        return Err(Error::Invalid {
                            id: id.into(),
                            reason: format!("a blind opens 0–100%, not {p}"),
                        });
                    }
                    BlindsLevel::Exact(*p)
                }
            };
            let target = match (device, room) {
                (Some(device), None) => {
                    BlindTarget::Device(parse_device_id(id, device, default_source)?)
                }
                (None, Some(room)) => BlindTarget::Room(parse_room_name(id, room)?),
                _ => {
                    return Err(Error::Invalid {
                        id: id.into(),
                        reason: "set_blinds needs a device or a room, not both".into(),
                    });
                }
            };
            Action::SetBlinds { target, level }
        }
        ActionDto::Notify {
            body,
            room,
            priority,
        } => Action::Notify {
            body: body.clone(),
            room: room.clone(),
            priority: parse_priority(id, priority.as_deref())?,
        },
        ActionDto::WakeUp {
            minutes,
            rooms,
            devices,
            except,
            brightness,
            start_kelvin,
            kelvin,
        } => {
            check_light(id, *brightness, *kelvin)?;
            check_light(id, None, *start_kelvin)?;
            if start_kelvin.is_some() && kelvin.is_none() {
                return Err(Error::Invalid {
                    id: id.into(),
                    reason: "a wake-up's start warmth needs an end warmth to go to".into(),
                });
            }
            let minutes = minutes.unwrap_or(WAKE_UP_MINUTES);
            if !(1..=MAX_WAKE_UP_MINUTES).contains(&minutes) {
                return Err(Error::Invalid {
                    id: id.into(),
                    reason: format!(
                        "a wake-up ramps over 1 to {MAX_WAKE_UP_MINUTES} minutes, not {minutes}"
                    ),
                });
            }
            let brightness = brightness.unwrap_or(100);
            if brightness == 0 {
                return Err(Error::Invalid {
                    id: id.into(),
                    reason: "a wake-up has to end above 0%".into(),
                });
            }
            let ids = |list: &[String]| {
                list.iter()
                    .map(|d| parse_device_id(id, d, default_source))
                    .collect::<Result<Vec<_>>>()
            };
            Action::WakeUp(WakePlan {
                minutes,
                rooms: rooms
                    .iter()
                    .map(|r| parse_room_name(id, r))
                    .collect::<Result<Vec<_>>>()?,
                devices: ids(devices)?,
                except: ids(except)?,
                brightness,
                start_kelvin: *start_kelvin,
                kelvin: *kelvin,
            })
        }
        ActionDto::Delay {
            hours,
            minutes,
            seconds,
        } => Action::Delay(wait_of(
            id,
            "a delay",
            &DurationDto {
                hours: *hours,
                minutes: *minutes,
                seconds: *seconds,
            },
        )?),
        ActionDto::If {
            conditions,
            then,
            otherwise,
        } => {
            if depth + 1 > MAX_DEPTH {
                return Err(Error::Invalid {
                    id: id.to_string(),
                    reason: format!("\"if\" may sit at most {MAX_DEPTH} deep"),
                });
            }
            if then.is_empty() {
                return Err(Error::Invalid {
                    id: id.to_string(),
                    reason: "an \"if\" needs something to do when it holds".into(),
                });
            }
            Action::If {
                conditions: conditions
                    .iter()
                    .map(|c| parse_condition(id, c, default_source, triggers))
                    .collect::<Result<Vec<_>>>()?,
                then: parse_actions(id, then, default_source, triggers, depth + 1)?,
                otherwise: parse_actions(id, otherwise, default_source, triggers, depth + 1)?,
            }
        }
    })
}

fn check_light(id: &str, brightness: Option<u8>, kelvin: Option<u16>) -> Result<()> {
    if let Some(b) = brightness
        && b > 100
    {
        return Err(Error::InvalidBrightness {
            id: id.to_string(),
            value: b,
        });
    }
    if let Some(k) = kelvin
        && !(2000..=6500).contains(&k)
    {
        return Err(Error::InvalidKelvin {
            id: id.to_string(),
            value: k,
        });
    }
    Ok(())
}

fn parse_device_id(id: &str, value: &str, default_source: &str) -> Result<DeviceId> {
    let s = if value.contains(':') {
        value.to_string()
    } else {
        format!("{default_source}:{value}")
    };
    DeviceId::parse(&s).map_err(|e| Error::InvalidDeviceId {
        id: id.to_string(),
        value: value.to_string(),
        source: e,
    })
}

fn parse_room_name(id: &str, value: &str) -> Result<RoomName> {
    RoomName::parse(value).map_err(|e| Error::InvalidRoom {
        id: id.to_string(),
        value: value.to_string(),
        source: e,
    })
}

fn parse_time(id: &str, value: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(value, "%H:%M").map_err(|e| Error::InvalidTime {
        id: id.to_string(),
        value: value.to_string(),
        source: e,
    })
}

fn parse_priority(id: &str, value: Option<&str>) -> Result<Priority> {
    match value.map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("routine") => Ok(Priority::Routine),
        Some("important") => Ok(Priority::Important),
        Some("urgent") => Ok(Priority::Urgent),
        Some(other) => Err(Error::InvalidPriority {
            id: id.to_string(),
            value: other.to_string(),
        }),
    }
}

fn parse_presence_filter(id: &str, value: Option<&str>) -> Result<Option<PresenceFilter>> {
    match value.map(|s| s.to_ascii_lowercase()).as_deref() {
        None => Ok(None),
        Some("home") => Ok(Some(PresenceFilter::Home)),
        Some("away") => Ok(Some(PresenceFilter::Away)),
        Some("unknown") => Ok(Some(PresenceFilter::Unknown)),
        Some(other) => Err(Error::InvalidPresenceState {
            id: id.to_string(),
            value: other.to_string(),
        }),
    }
}

fn presence_filter_matches(filter: PresenceFilter, state: PresenceState) -> bool {
    matches!(
        (filter, state),
        (PresenceFilter::Home, PresenceState::Home)
            | (PresenceFilter::Away, PresenceState::Away)
            | (PresenceFilter::Unknown, PresenceState::Unknown)
    )
}

/// Returns `true` if `t` falls inside the window `[after, before]`.
///
/// Handles both non-wrapping (10:00–14:00) and wrapping (21:00–06:00)
/// windows, including exact boundary times. If both bounds are `None`,
/// always returns `true`.
pub(crate) fn covers(after: Option<NaiveTime>, before: Option<NaiveTime>, t: NaiveTime) -> bool {
    match (after, before) {
        (None, None) => true,
        (Some(start), None) => t >= start,
        (None, Some(end)) => t <= end,
        (Some(start), Some(end)) => {
            if start <= end {
                t >= start && t <= end
            } else {
                t >= start || t <= end
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use niles_core::{Device, DeviceClass, DeviceName};
    use serde_json::json;

    fn device_id(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "z2m",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    fn rule(value: serde_json::Value) -> Result<Rule> {
        let dto: AutomationRuleDto = serde_json::from_value(value).unwrap();
        Rule::from_dto(&dto, "z2m")
    }

    fn waking(action: serde_json::Value) -> Result<Rule> {
        rule(serde_json::json!({
            "id": "wake",
            "triggers": [{ "type": "time", "at": "06:00" }],
            "actions": [action],
        }))
    }

    #[test]
    fn a_wake_up_takes_its_settings_or_the_routines() {
        let r = waking(serde_json::json!({ "do": "wake_up" })).unwrap();
        let Action::WakeUp(plan) = &r.actions[0] else {
            panic!("a wake-up");
        };
        assert_eq!(plan.minutes, WAKE_UP_MINUTES);
        assert_eq!(plan.brightness, 100);
        assert!(plan.rooms.is_empty() && plan.devices.is_empty());
        let r = waking(serde_json::json!({
            "do": "wake_up", "minutes": 20, "devices": ["bedroom/ceiling"], "kelvin": 3000
        }))
        .unwrap();
        let Action::WakeUp(plan) = &r.actions[0] else {
            panic!("a wake-up");
        };
        assert_eq!(plan.devices[0].to_string(), "z2m:bedroom/ceiling");
        assert_eq!(plan.kelvin, Some(3000));
    }

    #[test]
    fn a_wake_up_out_of_bounds_is_refused() {
        for bad in [
            serde_json::json!({ "do": "wake_up", "minutes": 0 }),
            serde_json::json!({ "do": "wake_up", "minutes": 121 }),
            serde_json::json!({ "do": "wake_up", "brightness": 0 }),
            serde_json::json!({ "do": "wake_up", "brightness": 101 }),
            serde_json::json!({ "do": "wake_up", "start_kelvin": 1500, "kelvin": 4000 }),
        ] {
            assert!(waking(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_wake_up_warms_from_its_start_to_its_end() {
        let r = waking(serde_json::json!({
            "do": "wake_up", "start_kelvin": 2200, "kelvin": 4000
        }))
        .unwrap();
        let Action::WakeUp(plan) = &r.actions[0] else {
            panic!("not a wake-up");
        };
        assert_eq!((plan.start_kelvin, plan.kelvin), (Some(2200), Some(4000)));
        let err = waking(serde_json::json!({ "do": "wake_up", "start_kelvin": 2200 }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("needs an end warmth"), "{err}");
    }

    #[test]
    fn a_rule_changed_is_a_different_fingerprint() {
        let one = waking(serde_json::json!({ "do": "wake_up", "minutes": 20 })).unwrap();
        let same = waking(serde_json::json!({ "do": "wake_up", "minutes": 20 })).unwrap();
        let other = waking(serde_json::json!({ "do": "wake_up", "minutes": 25 })).unwrap();
        assert_eq!(one.fingerprint, same.fingerprint);
        assert_ne!(one.fingerprint, other.fingerprint);
    }

    fn lights_off() -> serde_json::Value {
        json!({ "do": "set_room", "room": "office", "on": false })
    }

    fn door(open: bool) -> serde_json::Value {
        json!({ "type": "contact", "device": "office/door", "open": open })
    }

    fn state_event(id: &DeviceId, state: DeviceState) -> Event {
        Event::DeviceStateChanged {
            id: id.clone(),
            state,
        }
    }

    fn ctx(registry: &DeviceRegistry, now: DateTime<Utc>) -> Context<'_> {
        Context {
            registry,
            now,
            tz: Tz::UTC,
            location: None,
            presence: None,
            trigger: None,
        }
    }

    // ---- validation ------------------------------------------------

    #[test]
    fn rejects_bad_ids() {
        for bad in ["", "-bad", "bad-", "bad--id", "Bad"] {
            let r = rule(json!({ "id": bad, "triggers": [door(true)], "actions": [lights_off()] }));
            assert!(r.is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rejects_a_rule_without_a_trigger_or_an_action() {
        assert!(rule(json!({ "id": "r", "triggers": [], "actions": [lights_off()] })).is_err());
        assert!(matches!(
            rule(json!({ "id": "r", "triggers": [door(true)], "actions": [] })),
            Err(Error::NoActions { .. })
        ));
    }

    #[test]
    fn names_unnamed_triggers_in_order() {
        let mut dto: AutomationRuleDto = serde_json::from_value(json!({
            "id": "r",
            "triggers": [door(true), { "id": "t1", "type": "contact", "device": "office/door" }, door(false)],
            "actions": [lights_off()],
        }))
        .unwrap();
        assign_trigger_ids(&mut dto);
        let ids: Vec<_> = dto.triggers.iter().map(|t| t.id.clone().unwrap()).collect();
        assert_eq!(ids, ["t2", "t1", "t3"]);
        assert!(Rule::from_dto(&dto, "z2m").is_ok());
    }

    #[test]
    fn refuses_two_triggers_of_one_name_and_a_triggered_by_naming_none() {
        let twice = rule(json!({
            "id": "r",
            "triggers": [{ "id": "a", "type": "contact", "device": "office/door" }, { "id": "a", "type": "timer_fired" }],
            "actions": [lights_off()],
        }));
        assert!(twice.is_err());
        let gone = rule(json!({
            "id": "r",
            "triggers": [door(true)],
            "conditions": [{ "kind": "triggered_by", "triggers": ["t9"] }],
            "actions": [lights_off()],
        }));
        assert!(gone.unwrap_err().to_string().contains("t9"));
    }

    #[test]
    fn refuses_waits_out_of_bounds_and_deep_branches() {
        let long = rule(json!({
            "id": "r", "triggers": [door(true)],
            "actions": [{ "do": "delay", "hours": 25 }],
        }));
        assert!(long.is_err());
        let none = rule(json!({
            "id": "r",
            "triggers": [{ "type": "contact", "device": "office/door", "for": {} }],
            "actions": [lights_off()],
        }));
        assert!(none.is_err());
        let nested = |depth: usize| {
            let mut action = lights_off();
            for _ in 0..depth {
                action = json!({ "do": "if", "conditions": [], "then": [action] });
            }
            rule(json!({ "id": "r", "triggers": [door(true)], "actions": [action] }))
        };
        assert!(nested(3).is_ok());
        assert!(nested(4).is_err());
    }

    #[test]
    fn refuses_readings_that_can_never_hold() {
        let r = |above: Option<f64>, below: Option<f64>| {
            rule(json!({
                "id": "r",
                "triggers": [{ "type": "numeric", "device": "office/sensor", "measure": "temperature", "above": above, "below": below }],
                "actions": [lights_off()],
            }))
        };
        assert!(r(Some(24.0), None).is_ok());
        assert!(r(None, None).is_err());
        assert!(r(Some(25.0), Some(20.0)).is_err());
        assert!(
            rule(json!({
                "id": "r",
                "triggers": [{ "type": "numeric", "device": "office/sensor", "measure": "loud ness", "above": 1 }],
                "actions": [lights_off()],
            }))
            .is_err(),
            "no reading can be called that"
        );
    }

    #[test]
    fn any_other_valid_name_is_a_reading() {
        assert_eq!(
            Measure::parse("price"),
            Some(Measure::Reading("price".into()))
        );
        assert_eq!(
            Measure::parse(" CO2 "),
            Some(Measure::Reading("co2".into()))
        );
        assert_eq!(Measure::parse("lux"), Some(Measure::Illuminance));
        assert_eq!(Measure::parse("temperature"), Some(Measure::Temperature));
        assert_eq!(Measure::parse("co₂"), None);

        let mut state = DeviceState::default();
        state.readings.insert(
            "price".into(),
            niles_core::Reading::new(0.8, Some("kr/kWh".into())),
        );
        assert_eq!(Measure::Reading("price".into()).read(&state), Some(0.8));
        assert_eq!(Measure::Reading("power".into()).read(&state), None);
    }

    #[test]
    fn reads_clock_and_sun_triggers() {
        let r = rule(json!({
            "id": "r",
            "triggers": [
                { "type": "time", "at": "22:00", "days": ["mon", "fri"] },
                { "type": "sun", "event": "sunset", "offset_minutes": -30 }
            ],
            "actions": [lights_off()],
        }))
        .unwrap();
        assert!(matches!(r.triggers[0].trigger, Trigger::Time { .. }));
        assert!(r.uses_sun());
        assert!(
            rule(json!({
                "id": "r", "triggers": [{ "type": "time", "at": "25:00" }], "actions": [lights_off()],
            }))
            .is_err()
        );
    }

    #[test]
    fn keeps_brightness_and_kelvin_in_range() {
        let r = |action: serde_json::Value| {
            rule(json!({ "id": "r", "triggers": [door(true)], "actions": [action] }))
        };
        assert!(r(json!({ "do": "set_room", "room": "office", "brightness": 101 })).is_err());
        assert!(r(json!({ "do": "set_device", "device": "office/go", "kelvin": 1000 })).is_err());
        assert!(r(json!({ "do": "notify", "body": "x", "priority": "loud" })).is_err());
    }

    // ---- what triggers follow --------------------------------------

    #[test]
    fn blinds_are_set_by_room_or_one_at_a_time() {
        let blinds = |action: serde_json::Value| {
            rule(json!({
                "id": "r",
                "triggers": [{ "type": "sun", "event": "sunset" }],
                "actions": [action],
            }))
        };
        let r =
            blinds(json!({ "do": "set_blinds", "room": "living_room", "position": 0 })).unwrap();
        assert!(matches!(
            &r.actions[0],
            Action::SetBlinds {
                target: BlindTarget::Room(_),
                level: BlindsLevel::Exact(0)
            }
        ));
        assert!(
            blinds(json!({ "do": "set_blinds", "device": "office/blind", "position": 50 })).is_ok()
        );
        let r = blinds(json!({ "do": "set_blinds", "device": "office/blind", "position": "open" }))
            .unwrap();
        assert!(matches!(
            &r.actions[0],
            Action::SetBlinds {
                level: BlindsLevel::Open,
                ..
            }
        ));
        assert!(blinds(json!({ "do": "set_blinds", "position": 50 })).is_err());
        // 150 no longer reaches the rule: the DTO refuses it, naming the value.
        let err = serde_json::from_value::<ActionDto>(json!({
            "do": "set_blinds", "room": "office", "position": 150
        }))
        .unwrap_err();
        assert!(err.to_string().contains("150"), "{err}");
        // Built in Rust, where the deserializer never ran: still refused.
        let dto = ActionDto::SetBlinds {
            device: Some("office/blind".into()),
            room: None,
            position: BlindsPositionDto::Exact(150),
        };
        assert!(parse_action("r", &dto, "z2m", &[], 0).is_err());
    }

    #[test]
    fn a_motion_trigger_follows_the_sensor() {
        let sensor = device_id("hallway", "motion");
        let t = Trigger::Motion {
            device: sensor.clone(),
            detected: Some(false),
        };
        let seen = |motion: bool| DeviceState {
            motion: Some(motion),
            ..Default::default()
        };
        assert_eq!(
            t.observe(&state_event(&sensor, seen(false)), &DeviceRegistry::new()),
            Some(("z2m:hallway/motion".into(), true))
        );
        assert_eq!(
            t.observe(&state_event(&sensor, seen(true)), &DeviceRegistry::new()),
            Some(("z2m:hallway/motion".into(), false))
        );
        let lux_only = DeviceState {
            illuminance_lux: Some(40.0),
            ..Default::default()
        };
        assert_eq!(
            t.observe(&state_event(&sensor, lux_only), &DeviceRegistry::new()),
            None
        );
        assert!(t.follows_state());
    }

    #[test]
    fn lights_on_with_motion_and_off_after_a_quiet_spell_is_one_rule() {
        let r = rule(json!({
            "id": "hallway",
            "triggers": [
                { "id": "seen", "type": "motion", "device": "hallway/motion", "detected": true },
                { "id": "quiet", "type": "motion", "device": "hallway/motion", "detected": false, "for": { "minutes": 2 } }
            ],
            "conditions": [{ "kind": "numeric", "device": "hallway/motion", "measure": "illuminance", "below": 30 }],
            "actions": [lights_off()],
        }))
        .unwrap();
        assert_eq!(r.triggers[1].hold, Some(Duration::from_secs(120)));
        assert!(matches!(
            r.conditions[0],
            Condition::Numeric {
                measure: Measure::Illuminance,
                ..
            }
        ));
    }

    #[test]
    fn a_numeric_condition_reads_a_named_reading() {
        let r = rule(json!({
            "id": "r",
            "triggers": [door(true)],
            "conditions": [{ "kind": "numeric", "device": "office/air", "measure": "co2", "above": 1000 }],
            "actions": [lights_off()],
        }))
        .unwrap();
        let condition = &r.conditions[0];
        assert!(matches!(
            condition,
            Condition::Numeric { measure: Measure::Reading(name), .. } if name == "co2"
        ));

        let registry = DeviceRegistry::new();
        let air = |ppm: f64| {
            let mut state = DeviceState::default();
            state.readings.insert(
                "co2".into(),
                niles_core::Reading::new(ppm, Some("ppm".into())),
            );
            Device::new(device_id("office", "air"), state, DeviceClass::Sensor)
        };
        registry.upsert(air(1200.0));
        assert!(condition.evaluate(&ctx(&registry, Utc::now())));
        registry.upsert(air(600.0));
        assert!(!condition.evaluate(&ctx(&registry, Utc::now())));
        assert_eq!(
            condition.unmet(),
            "the co2 of z2m:office/air was out of range"
        );
        // A device that has never reported it does not pass.
        registry.upsert(Device::new(
            device_id("office", "air"),
            DeviceState::default(),
            DeviceClass::Sensor,
        ));
        assert!(!condition.evaluate(&ctx(&registry, Utc::now())));
    }

    #[test]
    fn a_motion_condition_reads_the_sensor() {
        let registry = DeviceRegistry::new();
        let sensor = device_id("kitchen", "motion");
        registry.upsert(Device::new(
            sensor.clone(),
            DeviceState {
                motion: Some(true),
                ..Default::default()
            },
            DeviceClass::Motion,
        ));
        let is = |detected: bool| Condition::MotionIs {
            device: sensor.clone(),
            detected,
        };
        assert!(is(true).evaluate(&ctx(&registry, Utc::now())));
        assert!(!is(false).evaluate(&ctx(&registry, Utc::now())));
    }

    fn sensor(registry: &DeviceRegistry, room: &str, motion: Option<bool>) -> DeviceId {
        let id = device_id(room, "motion");
        registry.upsert(Device::new(
            id.clone(),
            DeviceState {
                motion,
                ..Default::default()
            },
            DeviceClass::Motion,
        ));
        id
    }

    fn group_trigger(sensors: serde_json::Value, detected: Option<bool>) -> Result<Rule> {
        let mut trigger = json!({ "type": "motion", "for": { "minutes": 2 } });
        for (k, v) in sensors.as_object().unwrap() {
            trigger[k] = v.clone();
        }
        if let Some(detected) = detected {
            trigger["detected"] = json!(detected);
        }
        rule(json!({ "id": "r", "triggers": [trigger], "actions": [lights_off()] }))
    }

    #[test]
    fn a_motion_trigger_takes_several_sensors_as_one() {
        let r = group_trigger(
            json!({ "devices": ["hallway/motion", "kitchen/motion", "hallway/motion"], "rooms": ["office"] }),
            Some(false),
        )
        .unwrap();
        let Trigger::MotionGroup { group, detected } = &r.triggers[0].trigger else {
            panic!("a group");
        };
        assert!(!detected);
        assert_eq!(
            group.devices,
            [
                device_id("hallway", "motion"),
                device_id("kitchen", "motion")
            ],
            "a sensor named twice counts once"
        );
        assert_eq!(group.rooms, [RoomName::parse("office").unwrap()]);
        assert_eq!(r.triggers[0].hold, Some(Duration::from_secs(120)));
        assert!(r.triggers[0].trigger.follows_state());
    }

    #[test]
    fn a_list_of_one_sensor_is_that_sensor() {
        let r = group_trigger(json!({ "devices": ["hallway/motion"] }), Some(false)).unwrap();
        assert!(matches!(r.triggers[0].trigger, Trigger::Motion { .. }));
    }

    #[test]
    fn a_motion_trigger_names_its_sensors_one_way() {
        let err = |sensors: serde_json::Value, detected: Option<bool>| {
            group_trigger(sensors, detected).unwrap_err().to_string()
        };
        assert!(
            err(
                json!({ "device": "hallway/motion", "devices": ["kitchen/motion"] }),
                Some(false)
            )
            .contains("not both")
        );
        assert!(err(json!({}), Some(false)).contains("needs a sensor"));
        assert!(err(json!({ "devices": [] }), Some(false)).contains("needs a sensor"));
        assert!(
            err(
                json!({ "devices": ["hallway/motion", "kitchen/motion"] }),
                None
            )
            .contains("needs `detected`")
        );
        assert!(
            err(
                json!({ "devices": ["no slash", "kitchen/motion"] }),
                Some(false)
            )
            .contains("no slash")
        );
        assert!(err(json!({ "rooms": ["Not A Room"] }), Some(false)).contains("Not A Room"));
    }

    #[test]
    fn a_group_is_clear_only_once_every_sensor_is() {
        let registry = DeviceRegistry::new();
        let hallway = sensor(&registry, "hallway", Some(true));
        let kitchen = sensor(&registry, "kitchen", Some(false));
        let t = Trigger::MotionGroup {
            group: MotionGroup {
                devices: vec![hallway.clone(), kitchen.clone()],
                rooms: vec![],
            },
            detected: false,
        };
        let seen = |id: &DeviceId, motion: bool| {
            state_event(
                id,
                DeviceState {
                    motion: Some(motion),
                    ..Default::default()
                },
            )
        };
        // The kitchen repeating itself: the hallway still sees somebody.
        assert_eq!(
            t.observe(&seen(&kitchen, false), &registry),
            Some((GROUP_SUBJECT.into(), false))
        );
        // The hallway clearing, before the registry has caught up.
        assert_eq!(
            t.observe(&seen(&hallway, false), &registry),
            Some((GROUP_SUBJECT.into(), true))
        );
        // A report from somewhere else, or without motion, says nothing.
        assert_eq!(
            t.observe(&seen(&device_id("office", "motion"), true), &registry),
            None
        );
        let battery = state_event(
            &kitchen,
            DeviceState {
                battery_percent: Some(80),
                ..Default::default()
            },
        );
        assert_eq!(t.observe(&battery, &registry), None);

        let any = Trigger::MotionGroup {
            group: MotionGroup {
                devices: vec![hallway.clone(), kitchen.clone()],
                rooms: vec![],
            },
            detected: true,
        };
        assert_eq!(
            any.observe(&seen(&kitchen, false), &registry).map(|o| o.1),
            Some(true),
            "the hallway still sees somebody"
        );
        assert_eq!(
            any.observe(&seen(&hallway, false), &registry).map(|o| o.1),
            Some(false)
        );
    }

    #[test]
    fn a_sensor_not_heard_from_is_not_known_to_be_clear() {
        let registry = DeviceRegistry::new();
        let hallway = sensor(&registry, "hallway", Some(false));
        let kitchen = sensor(&registry, "kitchen", None);
        let group = MotionGroup {
            devices: vec![
                hallway.clone(),
                kitchen.clone(),
                device_id("attic", "motion"),
            ],
            rooms: vec![],
        };
        let clear = Trigger::MotionGroup {
            group: group.clone(),
            detected: false,
        };
        assert_eq!(
            clear.seed(&registry),
            [(GROUP_SUBJECT.to_string(), false)],
            "never reported, nor paired"
        );
        sensor(&registry, "kitchen", Some(false));
        assert!(!clear.seed(&registry)[0].1, "the attic is still not paired");

        let paired = Trigger::MotionGroup {
            group: MotionGroup {
                devices: vec![hallway.clone(), kitchen.clone()],
                rooms: vec![],
            },
            detected: false,
        };
        assert!(paired.seed(&registry)[0].1);
        // Out of reach, its last word does not count either way.
        registry.set_available(&kitchen, false);
        assert!(!paired.seed(&registry)[0].1);
        sensor(&registry, "kitchen", Some(true));
        registry.set_available(&kitchen, false);
        let any = Trigger::MotionGroup {
            group: MotionGroup {
                devices: vec![hallway, kitchen],
                rooms: vec![],
            },
            detected: true,
        };
        assert!(!any.seed(&registry)[0].1);
    }

    #[test]
    fn a_rooms_sensors_are_found_when_asked_so_a_new_one_counts() {
        let registry = DeviceRegistry::new();
        let t = Trigger::MotionGroup {
            group: MotionGroup {
                devices: vec![],
                rooms: vec![RoomName::parse("kitchen").unwrap()],
            },
            detected: false,
        };
        // A light in the room is not one of its motion sensors.
        registry.upsert(Device::new(
            device_id("kitchen", "ceiling"),
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        assert!(!t.seed(&registry)[0].1, "no sensor is no all-clear");
        sensor(&registry, "kitchen", Some(false));
        assert!(t.seed(&registry)[0].1);
        let radar = device_id("kitchen", "radar");
        registry.upsert(Device::new(
            radar.clone(),
            DeviceState {
                motion: Some(true),
                ..Default::default()
            },
            DeviceClass::Sensor,
        ));
        assert!(!t.seed(&registry)[0].1, "paired since, and sees somebody");
    }

    #[test]
    fn a_motion_condition_over_several_sensors_is_all_clear_or_any_motion() {
        let registry = DeviceRegistry::new();
        sensor(&registry, "hallway", Some(false));
        sensor(&registry, "kitchen", Some(true));
        let r = rule(json!({
            "id": "r",
            "triggers": [door(true)],
            "conditions": [
                { "kind": "motion_is", "devices": ["hallway/motion", "kitchen/motion"], "detected": false },
                { "kind": "motion_is", "devices": ["hallway/motion", "kitchen/motion"], "detected": true }
            ],
            "actions": [lights_off()],
        }))
        .unwrap();
        let (clear, any) = (&r.conditions[0], &r.conditions[1]);
        assert!(matches!(clear, Condition::MotionGroupIs { .. }));
        assert!(!clear.evaluate(&ctx(&registry, Utc::now())));
        assert!(any.evaluate(&ctx(&registry, Utc::now())));
        assert_eq!(
            clear.unmet(),
            "one of z2m:hallway/motion and z2m:kitchen/motion saw motion, or had not been heard from"
        );
        sensor(&registry, "kitchen", Some(false));
        assert!(clear.evaluate(&ctx(&registry, Utc::now())));
        assert!(!any.evaluate(&ctx(&registry, Utc::now())));
        assert_eq!(
            any.unmet(),
            "none of z2m:hallway/motion and z2m:kitchen/motion saw motion"
        );
    }

    #[test]
    fn a_contact_trigger_follows_the_door() {
        let door = device_id("office", "door");
        let t = Trigger::Contact {
            device: door.clone(),
            open: Some(true),
        };
        let open = DeviceState {
            open: Some(true),
            ..Default::default()
        };
        let shut = DeviceState {
            open: Some(false),
            ..Default::default()
        };
        assert_eq!(
            t.observe(&state_event(&door, open), &DeviceRegistry::new()),
            Some(("z2m:office/door".into(), true))
        );
        assert_eq!(
            t.observe(&state_event(&door, shut), &DeviceRegistry::new()),
            Some(("z2m:office/door".into(), false))
        );
        // A battery report without the contact says nothing about it.
        let battery = DeviceState {
            battery_percent: Some(90),
            ..Default::default()
        };
        assert_eq!(
            t.observe(&state_event(&door, battery), &DeviceRegistry::new()),
            None
        );
        assert!(t.follows_state());
    }

    #[test]
    fn a_numeric_trigger_holds_inside_its_range() {
        let sensor = device_id("office", "sensor");
        let t = Trigger::Numeric {
            device: sensor.clone(),
            measure: Measure::Temperature,
            above: Some(24.0),
            below: None,
        };
        let at = |c: f32| {
            state_event(
                &sensor,
                DeviceState {
                    temperature_celsius: Some(c),
                    ..Default::default()
                },
            )
        };
        assert_eq!(
            t.observe(&at(25.5), &DeviceRegistry::new()).map(|o| o.1),
            Some(true)
        );
        assert_eq!(
            t.observe(&at(24.0), &DeviceRegistry::new()).map(|o| o.1),
            Some(false)
        );
    }

    #[test]
    fn a_device_state_trigger_without_a_device_follows_every_one_in_the_room() {
        let t = Trigger::DeviceState {
            device: None,
            room: Some(RoomName::parse("kitchen").unwrap()),
            on: Some(true),
        };
        let on = DeviceState {
            on: Some(true),
            ..Default::default()
        };
        assert!(
            t.observe(
                &state_event(&device_id("kitchen", "lamp"), on.clone()),
                &DeviceRegistry::new()
            )
            .is_some()
        );
        assert!(
            t.observe(
                &state_event(&device_id("office", "lamp"), on),
                &DeviceRegistry::new()
            )
            .is_none()
        );
    }

    #[test]
    fn buttons_and_timers_are_moments() {
        let dimmer = device_id("office", "switch");
        let press = Trigger::DeviceAction {
            device: dimmer.clone(),
            action: Some("on_press".into()),
        };
        assert!(press.fires(&Event::DeviceAction {
            id: dimmer.clone(),
            action: "on_press".into()
        }));
        assert!(!press.fires(&Event::DeviceAction {
            id: dimmer,
            action: "off_press".into()
        }));
        assert!(!press.follows_state());
    }

    #[test]
    fn a_weekday_time_falls_only_on_its_days() {
        let t = Trigger::Time {
            at: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            days: Some(vec![Weekday::Mon]),
        };
        let monday = Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
        let tuesday = Utc.with_ymd_and_hms(2026, 10, 13, 9, 0, 0).unwrap();
        assert_eq!(
            t.due_on(monday, Tz::UTC, None).map(|d| d.time()),
            NaiveTime::from_hms_opt(22, 0, 0)
        );
        assert_eq!(t.due_on(tuesday, Tz::UTC, None), None);
    }

    #[test]
    fn a_sun_trigger_falls_on_a_whole_minute() {
        let t = Trigger::Sun {
            event: SunEvent::Sunset,
            offset_minutes: -30,
        };
        let tz: Tz = "Europe/Copenhagen".parse().unwrap();
        let day = Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
        let due = t.due_on(day, tz, Location::of(56.1572, 10.2107)).unwrap();
        assert_eq!(due.second(), 0);
        assert!((17..=18).contains(&due.hour()), "{due}");
        assert_eq!(t.due_on(day, tz, None), None);
    }

    // ---- conditions ------------------------------------------------

    #[test]
    fn time_of_day_wraps_past_midnight_and_knows_its_days() {
        let registry = DeviceRegistry::new();
        let window = Condition::TimeOfDay {
            after: NaiveTime::from_hms_opt(22, 0, 0),
            before: NaiveTime::from_hms_opt(6, 0, 0),
            days: None,
        };
        let night = Utc.with_ymd_and_hms(2026, 10, 12, 23, 30, 0).unwrap();
        let noon = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
        assert!(window.evaluate(&ctx(&registry, night)));
        assert!(!window.evaluate(&ctx(&registry, noon)));
        let weekends = Condition::TimeOfDay {
            after: None,
            before: None,
            days: Some(vec![Weekday::Sat, Weekday::Sun]),
        };
        assert!(!weekends.evaluate(&ctx(&registry, noon)), "a Monday");
    }

    #[test]
    fn the_sun_condition_knows_when_it_is_dark() {
        let registry = DeviceRegistry::new();
        let dark = Condition::Sun {
            after: Some((SunEvent::Sunset, 0)),
            before: Some((SunEvent::Sunrise, 0)),
        };
        let tz: Tz = "Europe/Copenhagen".parse().unwrap();
        let at = |h: u32| Context {
            registry: &registry,
            now: tz
                .with_ymd_and_hms(2026, 10, 12, h, 0, 0)
                .unwrap()
                .with_timezone(&Utc),
            tz,
            location: Location::of(56.1572, 10.2107),
            presence: None,
            trigger: None,
        };
        assert!(dark.evaluate(&at(21)));
        assert!(dark.evaluate(&at(5)));
        assert!(!dark.evaluate(&at(13)));
        // Nowhere to see the sun from: never.
        let nowhere = Context {
            location: None,
            ..at(21)
        };
        assert!(!dark.evaluate(&nowhere));
    }

    #[test]
    fn device_conditions_read_the_registry_and_fail_safe() {
        let registry = DeviceRegistry::new();
        let lamp = device_id("office", "go");
        let sensor = device_id("office", "sensor");
        registry.upsert(Device::new(
            lamp.clone(),
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        registry.upsert(Device::new(
            sensor.clone(),
            DeviceState {
                temperature_celsius: Some(22.0),
                ..Default::default()
            },
            DeviceClass::Sensor,
        ));
        let now = Utc::now();
        assert!(
            Condition::DeviceIs {
                device: lamp,
                on: true
            }
            .evaluate(&ctx(&registry, now))
        );
        assert!(
            !Condition::DeviceIs {
                device: device_id("office", "gone"),
                on: false
            }
            .evaluate(&ctx(&registry, now))
        );
        assert!(
            Condition::Numeric {
                device: sensor,
                measure: Measure::Temperature,
                above: Some(20.0),
                below: Some(23.0),
            }
            .evaluate(&ctx(&registry, now))
        );
    }

    #[test]
    fn triggered_by_names_the_trigger_that_started_it() {
        let registry = DeviceRegistry::new();
        let only_first = Condition::TriggeredBy(vec!["t1".into()]);
        let started = |t: &'static str| Context {
            trigger: Some(t),
            ..ctx(&registry, Utc::now())
        };
        assert!(only_first.evaluate(&started("t1")));
        assert!(!only_first.evaluate(&started("t2")));
        assert_eq!(only_first.unmet(), "another trigger started it");
    }

    #[test]
    fn presence_is_needs_to_know() {
        let registry = DeviceRegistry::new();
        let home = Condition::PresenceIs(PresenceFilter::Home);
        assert!(!home.evaluate(&ctx(&registry, Utc::now())));
        let known = Context {
            presence: Some(PresenceState::Home),
            ..ctx(&registry, Utc::now())
        };
        assert!(home.evaluate(&known));
    }

    #[test]
    fn says_what_was_done_and_for_how_long() {
        assert_eq!(say_duration(Duration::from_secs(5 * 60)), "5 minutes");
        assert_eq!(
            say_duration(Duration::from_secs(3600 + 60)),
            "1 hour 1 minute"
        );
        assert_eq!(
            Action::SetRoom {
                room: RoomName::parse("office").unwrap(),
                on: Some(false),
                brightness: None,
                kelvin: None,
            }
            .done(),
            "set the office lights off"
        );
        assert_eq!(
            Action::SetBlinds {
                target: BlindTarget::Room(RoomName::parse("living_room").unwrap()),
                level: BlindsLevel::Open,
            }
            .done(),
            "opened the living_room blinds"
        );
        assert_eq!(
            Action::SetBlinds {
                target: BlindTarget::Room(RoomName::parse("living_room").unwrap()),
                level: BlindsLevel::Exact(40),
            }
            .done(),
            "set the living_room blinds to 40% open"
        );
    }

    #[test]
    fn a_sunset_rule_waiting_for_after_sunset_never_runs() {
        let registry = DeviceRegistry::new();
        let tz: Tz = "Europe/Copenhagen".parse().unwrap();
        let here = Location::of(56.1572, 10.2107);
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
        let curtains = |trigger: serde_json::Value, conditions: serde_json::Value| {
            rule(json!({
                "id": "curtains",
                "triggers": [trigger],
                "conditions": conditions,
                "actions": [{ "do": "notify", "body": "Close the curtains" }],
            }))
            .unwrap()
        };
        let wrong = curtains(
            json!({ "type": "sun", "event": "sunset" }),
            json!([{ "kind": "sun", "after": "sunset", "after_offset_minutes": 30 }]),
        );
        assert!(wrong.never_runs(&registry, now, tz, here));
        let right = curtains(
            json!({ "type": "sun", "event": "sunset", "offset_minutes": 30 }),
            json!([]),
        );
        assert!(!right.never_runs(&registry, now, tz, here));
        let evening = curtains(
            json!({ "type": "time", "at": "21:00" }),
            json!([{ "kind": "sun", "after": "sunset" }]),
        );
        assert!(!evening.never_runs(&registry, now, tz, here));
    }

    #[test]
    fn the_old_single_trigger_still_reads() {
        let r =
            rule(json!({ "id": "r", "trigger": door(true), "actions": [lights_off()] })).unwrap();
        assert_eq!(r.triggers.len(), 1);
        assert_eq!(r.triggers[0].id, "t1");
    }
}
