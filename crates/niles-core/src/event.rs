//! Internal pub/sub bus and event types.

use crate::device::{Device, DeviceId, DeviceState};
use tokio::sync::broadcast;

/// Resolved household presence state. Mirrors `niles_presence::HomeState`
/// — kept local to `niles-core` so the bus can carry presence transitions
/// without an upward dep on `niles-presence`. The conversion lives in
/// `niles-presence`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresenceState {
    Home,
    Away,
    Unknown,
}

/// Events that flow on the internal bus.
///
/// `#[non_exhaustive]` lets new variants be added without breaking
/// downstream matches.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// A device was discovered or re-discovered by an upstream source.
    DeviceAdded { device: Device },
    /// A device disappeared from its upstream source.
    DeviceRemoved { id: DeviceId },
    /// A device's state changed (from an upstream report or an in-flight command).
    DeviceStateChanged { id: DeviceId, state: DeviceState },
    /// Transient event from a button-style device. Not state — fires
    /// once per press / hold-repeat, never replayed.
    DeviceAction {
        id: DeviceId,
        /// Z2M-format action string ("on_press", "up_hold", etc.).
        /// Kept as `String` to avoid coupling niles-core to the Z2M
        /// vocabulary; consumers parse what they care about.
        action: String,
    },
    /// The household's resolved presence state changed. Emitted by the
    /// presence subsystem only on a genuine transition (not per poll).
    PresenceChanged { state: PresenceState },
    /// A timer registered via `Intent::TimerSet` has reached its
    /// `expires_at`. Emitted once when the driver flips the entry from
    /// `Pending` to `Ringing`. Future consumer: satellite alarm playback.
    TimerFired {
        /// The `TimerId.0` value — kept as a raw `u64` to avoid an
        /// upward dep on `niles-scheduler`.
        id: u64,
        /// Canonical (trim + lowercase + underscore) name, or `None`
        /// if the user didn't name the timer.
        name: Option<String>,
        /// Satellite (peer) `SocketAddr` that issued the original
        /// `TimerSet`. Used later for two-stage escalation.
        origin: std::net::SocketAddr,
    },
}

/// Bounded pub/sub bus backed by `tokio::sync::broadcast`.
///
/// Cloning is cheap and produces a new handle to the same channel.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish an event. Returns the number of active receivers that
    /// will see it. Slow subscribers lose events rather than block the
    /// producer.
    pub fn publish(&self, event: Event) -> usize {
        self.tx.send(event).unwrap_or(0)
    }

    /// Subscribe to future events. Subscribers do not see events
    /// published before they subscribed.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceName, RoomName};

    fn fixture_id() -> DeviceId {
        DeviceId::new(
            "z2m",
            RoomName::parse("kitchen").unwrap(),
            DeviceName::parse("ceiling_light").unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn publish_reaches_subscriber() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        bus.publish(Event::DeviceRemoved { id: fixture_id() });
        match rx.recv().await.unwrap() {
            Event::DeviceRemoved { id } => assert_eq!(id, fixture_id()),
            _ => panic!("expected DeviceRemoved"),
        }
    }

    #[tokio::test]
    async fn multiple_subscribers_each_see_event() {
        let bus = EventBus::default();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        bus.publish(Event::DeviceRemoved { id: fixture_id() });
        rx1.recv().await.unwrap();
        rx2.recv().await.unwrap();
    }

    #[test]
    fn publish_with_no_subscribers_does_not_panic() {
        let bus = EventBus::default();
        let count = bus.publish(Event::DeviceRemoved { id: fixture_id() });
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn publish_reaches_subscriber_device_action() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        bus.publish(Event::DeviceAction {
            id: fixture_id(),
            action: "on_press".to_string(),
        });
        match rx.recv().await.unwrap() {
            Event::DeviceAction { id, action } => {
                assert_eq!(id, fixture_id());
                assert_eq!(action, "on_press");
            }
            _ => panic!("expected DeviceAction"),
        }
    }

    #[tokio::test]
    async fn publish_reaches_subscriber_timer_fired() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        let origin: std::net::SocketAddr = "127.0.0.1:9999".parse().unwrap();
        bus.publish(Event::TimerFired {
            id: 42,
            name: Some("pasta".into()),
            origin,
        });
        match rx.recv().await.unwrap() {
            Event::TimerFired {
                id,
                name,
                origin: o,
            } => {
                assert_eq!(id, 42);
                assert_eq!(name.as_deref(), Some("pasta"));
                assert_eq!(o, origin);
            }
            _ => panic!("expected TimerFired"),
        }
    }

    #[tokio::test]
    async fn publish_reaches_subscriber_presence_changed() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        bus.publish(Event::PresenceChanged {
            state: PresenceState::Away,
        });
        match rx.recv().await.unwrap() {
            Event::PresenceChanged { state } => assert_eq!(state, PresenceState::Away),
            _ => panic!("expected PresenceChanged"),
        }
    }

    #[test]
    fn presence_state_clone_eq() {
        let a = PresenceState::Home;
        let b = a;
        assert_eq!(a, b);
        assert_eq!(PresenceState::Away, PresenceState::Away);
        assert_eq!(PresenceState::Unknown, PresenceState::Unknown);
        assert_ne!(PresenceState::Home, PresenceState::Away);
        assert_ne!(PresenceState::Home, PresenceState::Unknown);
        assert_ne!(PresenceState::Away, PresenceState::Unknown);
    }
}
