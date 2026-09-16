//! Lights that follow the house being empty or not.
//!
//! Not a rule, which is why it is not one. A rule names a device; this
//! is about whichever lights happen to be on when the last person
//! leaves, and that set is only known at the moment it matters. It
//! shares the engine's `DeviceSink` and registry because it is the same
//! job — deciding what to send and sending it — done from a different
//! question.

use crate::engine::DeviceSink;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus, PresenceState};
use std::sync::Arc;
use tokio::task::JoinHandle;

/// What the house should do as people come and go.
pub struct PresenceLights {
    registry: Arc<DeviceRegistry>,
    sink: Arc<dyn DeviceSink>,
    /// Turn off whatever is on when the last person leaves.
    off_when_away: bool,
    /// Turn these on when somebody comes back.
    on_when_home: Vec<DeviceId>,
}

impl PresenceLights {
    pub fn new(
        registry: Arc<DeviceRegistry>,
        sink: Arc<dyn DeviceSink>,
        off_when_away: bool,
        on_when_home: Vec<DeviceId>,
    ) -> Self {
        Self {
            registry,
            sink,
            off_when_away,
            on_when_home,
        }
    }

    /// Whether either half is switched on, so the caller can decline to
    /// subscribe at all rather than run a task that does nothing.
    pub fn wanted(&self) -> bool {
        self.off_when_away || !self.on_when_home.is_empty()
    }

    /// React to the house emptying or filling.
    pub async fn handle(&self, state: PresenceState) {
        match state {
            PresenceState::Away if self.off_when_away => self.all_off().await,
            PresenceState::Home => self.arrival().await,
            // `Unknown` is the source having nothing to say, which is
            // not the same as an empty house and must not turn the
            // lights off in one that isn't.
            _ => {}
        }
    }

    /// Everything that is on, turned off.
    ///
    /// Only what is reporting itself on: a light that has never been
    /// heard from gets no command, because "off" sent to a device whose
    /// state is unknown is a guess dressed as an instruction — and a
    /// bulb somebody unplugged would have Niles talking to it forever.
    async fn all_off(&self) {
        let lit: Vec<DeviceId> = self
            .registry
            .list_all()
            .into_iter()
            .filter(|d| d.state.on == Some(true))
            .map(|d| d.id.clone())
            .collect();

        if lit.is_empty() {
            tracing::debug!("everyone left; nothing was on");
            return;
        }
        tracing::info!("everyone left; turning off {} light(s)", lit.len());
        for id in lit {
            self.sink
                .set(
                    &id,
                    &DeviceState {
                        on: Some(false),
                        ..Default::default()
                    },
                )
                .await;
        }
    }

    /// The named lights, turned on.
    ///
    /// Only the ones that are off. Sending `on` to a light already on
    /// would be a command nobody asked for, and the curve would read it
    /// as somebody setting a level by hand.
    async fn arrival(&self) {
        for id in &self.on_when_home {
            if self.registry.get(id).and_then(|d| d.state.on) == Some(true) {
                continue;
            }
            tracing::info!("somebody is home; turning on {id}");
            self.sink
                .set(
                    id,
                    &DeviceState {
                        on: Some(true),
                        ..Default::default()
                    },
                )
                .await;
        }
    }

    /// Listen for the house changing hands.
    pub fn spawn(self: Arc<Self>, bus: EventBus) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut rx = bus.subscribe();
            loop {
                match rx.recv().await {
                    Ok(Event::PresenceChanged { state }) => self.handle(state).await,
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("presence lights lagged by {n} events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{Device, DeviceClass};
    use tokio::sync::Mutex;

    struct RecordingSink {
        calls: Mutex<Vec<(DeviceId, DeviceState)>>,
    }

    #[async_trait::async_trait]
    impl DeviceSink for RecordingSink {
        async fn set(&self, device: &DeviceId, desired: &DeviceState) {
            self.calls
                .lock()
                .await
                .push((device.clone(), desired.clone()));
        }
    }

    fn id(room: &str, name: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:{room}/{name}")).expect("valid")
    }

    /// `on: None` is a light that has never reported.
    fn light(room: &str, name: &str, on: Option<bool>) -> Device {
        Device::new(
            id(room, name),
            DeviceState {
                on,
                ..Default::default()
            },
            DeviceClass::Light,
        )
    }

    struct Fixture {
        lights: Arc<PresenceLights>,
        sink: Arc<RecordingSink>,
    }

    fn fixture(devices: Vec<Device>, off_when_away: bool, on_when_home: Vec<DeviceId>) -> Fixture {
        let registry = Arc::new(DeviceRegistry::new());
        for device in devices {
            registry.upsert(device);
        }
        let sink = Arc::new(RecordingSink {
            calls: Mutex::new(Vec::new()),
        });
        Fixture {
            lights: Arc::new(PresenceLights::new(
                registry,
                sink.clone(),
                off_when_away,
                on_when_home,
            )),
            sink,
        }
    }

    #[tokio::test]
    async fn an_empty_house_has_its_lights_turned_off() {
        let f = fixture(
            vec![
                light("kitchen", "ceiling", Some(true)),
                light("office", "lamp", Some(true)),
            ],
            true,
            vec![],
        );
        f.lights.handle(PresenceState::Away).await;

        let calls = f.sink.calls.lock().await.clone();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(_, s)| s.on == Some(false)));
    }

    #[tokio::test]
    async fn a_light_already_off_is_left_alone() {
        // And so is one that has never said. "Off" to a device whose
        // state is unknown is a guess dressed as an instruction.
        let f = fixture(
            vec![
                light("kitchen", "ceiling", Some(false)),
                light("hall", "strip", None),
            ],
            true,
            vec![],
        );
        f.lights.handle(PresenceState::Away).await;
        assert!(f.sink.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn nothing_happens_when_it_was_not_asked_for() {
        let f = fixture(vec![light("kitchen", "ceiling", Some(true))], false, vec![]);
        f.lights.handle(PresenceState::Away).await;
        assert!(f.sink.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn an_unknown_house_is_not_an_empty_one() {
        // The source having nothing to say must never read as everyone
        // having left — that is the failure that turns the lights off
        // on somebody sitting in the room.
        let f = fixture(vec![light("kitchen", "ceiling", Some(true))], true, vec![]);
        f.lights.handle(PresenceState::Unknown).await;
        assert!(f.sink.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn coming_home_turns_on_the_named_lights_only() {
        let f = fixture(
            vec![
                light("hall", "lamp", Some(false)),
                light("office", "lamp", Some(false)),
            ],
            true,
            vec![id("hall", "lamp")],
        );
        f.lights.handle(PresenceState::Home).await;

        let calls = f.sink.calls.lock().await.clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, id("hall", "lamp"));
        assert_eq!(calls[0].1.on, Some(true));
    }

    #[tokio::test]
    async fn a_light_that_is_already_on_is_not_told_so_again() {
        // The curve reads a level set by hand as a reason to stop
        // touching a light; a command nobody asked for would buy that.
        let f = fixture(
            vec![light("hall", "lamp", Some(true))],
            false,
            vec![id("hall", "lamp")],
        );
        f.lights.handle(PresenceState::Home).await;
        assert!(f.sink.calls.lock().await.is_empty());
    }

    #[test]
    fn a_task_is_only_worth_running_when_something_is_switched_on() {
        let registry = Arc::new(DeviceRegistry::new());
        let sink = Arc::new(RecordingSink {
            calls: Mutex::new(Vec::new()),
        });
        let nothing = PresenceLights::new(registry.clone(), sink.clone(), false, vec![]);
        assert!(!nothing.wanted());
        let something = PresenceLights::new(registry, sink, false, vec![id("hall", "lamp")]);
        assert!(something.wanted());
    }
}
