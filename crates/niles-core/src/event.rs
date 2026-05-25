//! Internal pub/sub bus and event types.

use crate::device::{Device, DeviceId, DeviceState};
use tokio::sync::broadcast;

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
}
