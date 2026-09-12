//! What "it" refers to.
//!
//! "Turn off the office light" and then "turn it back on" is the most
//! natural thing anyone says to a house, and resolving the second one
//! needs the first. Without this the follow-up has no Tier 0 pattern
//! at all, so it escalates to the LLM — which is slower, costs a few
//! thousand tokens, and on a rate-limited account often just fails.
//!
//! Scoped per origin room rather than per target: "it" means whatever
//! *you* last changed from where you are standing, even if what you
//! changed was in another room.

use niles_core::{Device, RoomName};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long "it" keeps meaning the same thing.
///
/// Long enough to cover walking across the room and changing your mind;
/// short enough that tomorrow morning's "turn it on" doesn't act on
/// last night's lamp.
const DEFAULT_TTL: Duration = Duration::from_secs(10 * 60);

struct Remembered {
    devices: Vec<Device>,
    /// What to call them if we have to say it back: "the office light".
    spoken: String,
    at: Instant,
}

pub(crate) struct LastTarget {
    /// Keyed by the room the request came from; `None` for a satellite
    /// with no room configured, which shares one slot.
    by_origin: Mutex<HashMap<Option<String>, Remembered>>,
    ttl: Duration,
}

impl Default for LastTarget {
    fn default() -> Self {
        Self::new(DEFAULT_TTL)
    }
}

impl LastTarget {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            by_origin: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    pub(crate) fn remember(&self, origin: Option<&RoomName>, spoken: &str, devices: &[Device]) {
        self.remember_at(origin, spoken, devices, Instant::now());
    }

    /// The devices "it" refers to, or `None` when nothing was said
    /// recently enough for the word to mean anything.
    pub(crate) fn resolve(&self, origin: Option<&RoomName>) -> Option<(String, Vec<Device>)> {
        self.resolve_at(origin, Instant::now())
    }

    fn remember_at(
        &self,
        origin: Option<&RoomName>,
        spoken: &str,
        devices: &[Device],
        now: Instant,
    ) {
        if devices.is_empty() {
            return;
        }
        let mut guard = self.lock();
        guard.insert(
            key(origin),
            Remembered {
                devices: devices.to_vec(),
                spoken: spoken.to_string(),
                at: now,
            },
        );
    }

    fn resolve_at(&self, origin: Option<&RoomName>, now: Instant) -> Option<(String, Vec<Device>)> {
        let guard = self.lock();
        let remembered = guard.get(&key(origin))?;
        if now.duration_since(remembered.at) > self.ttl {
            return None;
        }
        Some((remembered.spoken.clone(), remembered.devices.clone()))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Option<String>, Remembered>> {
        self.by_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn key(origin: Option<&RoomName>) -> Option<String> {
    origin.map(|r| r.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{DeviceClass, DeviceId, DeviceState};

    fn device(id: &str) -> Device {
        Device::new(
            DeviceId::parse(id).unwrap(),
            DeviceState::default(),
            DeviceClass::Light,
        )
    }

    fn room(name: &str) -> RoomName {
        RoomName::parse(name).unwrap()
    }

    #[test]
    fn it_means_what_you_last_changed() {
        let last = LastTarget::default();
        let office = room("office");
        last.remember(
            Some(&office),
            "the office light",
            &[device("z2m:office/light")],
        );

        let (spoken, devices) = last.resolve(Some(&office)).expect("remembered");
        assert_eq!(spoken, "the office light");
        assert_eq!(devices.len(), 1);
    }

    #[test]
    fn it_means_nothing_before_anything_was_said() {
        let last = LastTarget::default();
        assert!(last.resolve(Some(&room("office"))).is_none());
    }

    #[test]
    fn it_stops_meaning_anything_after_a_while() {
        // Otherwise tomorrow morning's "turn it on" acts on last
        // night's lamp.
        let last = LastTarget::new(Duration::from_secs(60));
        let office = room("office");
        let start = Instant::now();
        last.remember_at(
            Some(&office),
            "the office light",
            &[device("z2m:office/light")],
            start,
        );

        assert!(
            last.resolve_at(Some(&office), start + Duration::from_secs(30))
                .is_some()
        );
        assert!(
            last.resolve_at(Some(&office), start + Duration::from_secs(90))
                .is_none()
        );
    }

    #[test]
    fn each_room_remembers_its_own() {
        // Said in the kitchen, "it" is not the office lamp somebody
        // else was talking to.
        let last = LastTarget::default();
        last.remember(
            Some(&room("office")),
            "the office light",
            &[device("z2m:office/light")],
        );
        assert!(last.resolve(Some(&room("kitchen"))).is_none());
    }

    #[test]
    fn nothing_is_not_worth_remembering() {
        let last = LastTarget::default();
        last.remember(Some(&room("office")), "nothing", &[]);
        assert!(last.resolve(Some(&room("office"))).is_none());
    }
}
