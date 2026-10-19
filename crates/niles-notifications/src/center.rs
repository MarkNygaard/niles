//! Notification center: in-memory ring buffer + delivery orchestration.

use crate::model::{DeliveryOutcome, Notification, Priority};
use crate::quiet::QuietHoursConfig;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportOutcome {
    Completed,
    Partial,
    Failed,
}

/// Trait for concrete delivery backends (Wyoming, push, etc.).
#[async_trait]
pub trait NotificationDelivery: Send + Sync {
    /// Complete transport to the selected audience; this does not acknowledge playback.
    async fn deliver(&self, text: &str, room: Option<&str>, priority: Priority)
    -> TransportOutcome;
}

/// Where each notification is kept so "what did you say?" still knows
/// after a restart, which every deploy is.
pub trait NotificationHistory: Send + Sync {
    /// Keep one. Called on the delivery path, so it hands the write off
    /// rather than waiting for it.
    fn keep(&self, notification: &Notification);
}

pub struct NotificationCenter {
    buffer: Mutex<VecDeque<Notification>>,
    capacity: usize,
    quiet: Option<QuietHoursConfig>,
    delivery: Option<Arc<dyn NotificationDelivery>>,
    history: Option<Arc<dyn NotificationHistory>>,
}

impl NotificationCenter {
    /// Create a new center with the given ring-buffer capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "NotificationCenter capacity must be > 0");
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            quiet: None,
            delivery: None,
            history: None,
        }
    }

    /// Attach quiet-hours configuration.
    pub fn with_quiet_hours(mut self, quiet: QuietHoursConfig) -> Self {
        self.quiet = Some(quiet);
        self
    }

    /// Attach a delivery backend.
    pub fn with_delivery(mut self, delivery: Arc<dyn NotificationDelivery>) -> Self {
        self.delivery = Some(delivery);
        self
    }

    /// Pick up where the last run left off: `earlier` (newest first, as
    /// the history reads them back) fills the buffer, and every new one
    /// is handed to `history`.
    pub fn with_history(
        mut self,
        earlier: Vec<Notification>,
        history: Arc<dyn NotificationHistory>,
    ) -> Self {
        {
            let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
            for n in earlier.into_iter().take(self.capacity).rev() {
                buffer.push_back(n);
            }
        }
        self.history = Some(history);
        self
    }

    /// Set or replace the delivery backend after construction.
    pub fn set_delivery(&mut self, delivery: Arc<dyn NotificationDelivery>) {
        self.delivery = Some(delivery);
    }

    /// Deliver a notification, applying quiet-hours suppression and
    /// recording the outcome in the ring buffer.
    ///
    /// * `Routine` → suppressed if quiet hours are active (floored to
    ///   `Important` and recorded as `Suppressed`).
    /// * `Important` → delivered even during quiet hours.
    /// * `Urgent` → always delivered.
    pub async fn deliver(
        &self,
        text: impl Into<String>,
        room: Option<String>,
        priority: Priority,
    ) -> Notification {
        let text = text.into();
        let now = Utc::now();
        let id = generate_id(&now);

        let (effective_priority, outcome) =
            if priority != Priority::Urgent && self.quiet_hours_active(now) {
                let floored = priority.quiet_floor();
                if floored != priority {
                    (floored, DeliveryOutcome::Suppressed)
                } else {
                    (
                        priority,
                        self.try_deliver(&text, room.as_deref(), priority).await,
                    )
                }
            } else {
                (
                    priority,
                    self.try_deliver(&text, room.as_deref(), priority).await,
                )
            };

        let notification = Notification {
            id,
            text,
            priority: effective_priority,
            room,
            outcome,
            created_at: now,
        };

        let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        if buffer.len() >= self.capacity {
            buffer.pop_front();
        }
        buffer.push_back(notification.clone());
        drop(buffer);
        if let Some(history) = &self.history {
            history.keep(&notification);
        }
        notification
    }

    /// Return the most recent notifications, newest first.
    pub fn recent(&self, limit: usize) -> Vec<Notification> {
        let buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        buffer.iter().rev().take(limit).cloned().collect()
    }

    fn quiet_hours_active(&self, now: chrono::DateTime<Utc>) -> bool {
        self.quiet
            .as_ref()
            .map(|q| q.is_active(now))
            .unwrap_or(false)
    }

    async fn try_deliver(
        &self,
        text: &str,
        room: Option<&str>,
        priority: Priority,
    ) -> DeliveryOutcome {
        match &self.delivery {
            Some(d) => match d.deliver(text, room, priority).await {
                TransportOutcome::Completed => DeliveryOutcome::Delivered,
                TransportOutcome::Partial => DeliveryOutcome::Partial,
                TransportOutcome::Failed => DeliveryOutcome::Failed,
            },
            None => DeliveryOutcome::Failed,
        }
    }
}

fn generate_id(now: &chrono::DateTime<Utc>) -> String {
    let suffix: u32 = rand::random();
    format!("{}-{:08x}", now.timestamp_millis(), suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct GatedDelivery {
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[async_trait]
    impl NotificationDelivery for GatedDelivery {
        async fn deliver(&self, _: &str, _: Option<&str>, _: Priority) -> TransportOutcome {
            self.started.notify_one();
            self.release.notified().await;
            TransportOutcome::Partial
        }
    }

    #[tokio::test]
    async fn pending_transport_is_not_recorded_and_partial_is_recorded_once() {
        let backend = Arc::new(GatedDelivery {
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let kept = Arc::new(Kept::default());
        let center = Arc::new(
            NotificationCenter::new(10)
                .with_delivery(backend.clone())
                .with_history(Vec::new(), kept.clone()),
        );
        let task = {
            let center = center.clone();
            tokio::spawn(async move { center.deliver("hello", None, Priority::Important).await })
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.started.notified(),
        )
        .await
        .unwrap();
        assert!(!task.is_finished());
        assert!(center.recent(10).is_empty());
        backend.release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.outcome, DeliveryOutcome::Partial);
        let history = center.recent(10);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, result.id);
        assert_eq!(history[0].outcome, DeliveryOutcome::Partial);
        let persisted = kept.0.lock().unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].id, result.id);
    }

    struct CountingDelivery {
        calls: AtomicUsize,
        succeed: bool,
    }

    impl CountingDelivery {
        fn new(succeed: bool) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                succeed,
            }
        }
    }

    #[async_trait]
    impl NotificationDelivery for CountingDelivery {
        async fn deliver(
            &self,
            _text: &str,
            _room: Option<&str>,
            _priority: Priority,
        ) -> TransportOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.succeed {
                TransportOutcome::Completed
            } else {
                TransportOutcome::Failed
            }
        }
    }

    /// Quiet from an hour ago until an hour from now, so the test is
    /// inside the window whenever it runs; a fixed 00:00–23:00 left
    /// every run in the last hour of the UTC day outside it.
    fn quiet_now() -> QuietHoursConfig {
        use chrono::Timelike;
        let hour = Utc::now().hour();
        QuietHoursConfig {
            enabled: true,
            window: Some(crate::quiet::QuietWindow::new(
                chrono::NaiveTime::from_hms_opt((hour + 23) % 24, 0, 0).unwrap(),
                chrono::NaiveTime::from_hms_opt((hour + 2) % 24, 0, 0).unwrap(),
            )),
            timezone: Some(chrono_tz::Tz::UTC),
        }
    }

    #[tokio::test]
    async fn delivers_routine_when_not_quiet() {
        let delivery = Arc::new(CountingDelivery::new(true));
        let center = NotificationCenter::new(10).with_delivery(delivery.clone());
        let n = center.deliver("hello", None, Priority::Routine).await;
        assert_eq!(n.outcome, DeliveryOutcome::Delivered);
        assert_eq!(n.priority, Priority::Routine);
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn suppresses_routine_during_quiet_hours() {
        let delivery = Arc::new(CountingDelivery::new(true));
        let center = NotificationCenter::new(10)
            .with_delivery(delivery.clone())
            .with_quiet_hours(quiet_now());
        let n = center.deliver("shh", None, Priority::Routine).await;
        assert_eq!(n.outcome, DeliveryOutcome::Suppressed);
        assert_eq!(n.priority, Priority::Important);
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn delivers_important_during_quiet_hours() {
        let delivery = Arc::new(CountingDelivery::new(true));
        let center = NotificationCenter::new(10)
            .with_delivery(delivery.clone())
            .with_quiet_hours(quiet_now());
        let n = center.deliver("hey", None, Priority::Important).await;
        assert_eq!(n.outcome, DeliveryOutcome::Delivered);
        assert_eq!(n.priority, Priority::Important);
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn delivers_urgent_unconditionally() {
        let delivery = Arc::new(CountingDelivery::new(true));
        let center = NotificationCenter::new(10)
            .with_delivery(delivery.clone())
            .with_quiet_hours(quiet_now());
        let n = center.deliver("fire", None, Priority::Urgent).await;
        assert_eq!(n.outcome, DeliveryOutcome::Delivered);
        assert_eq!(n.priority, Priority::Urgent);
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn records_failed_delivery() {
        let delivery = Arc::new(CountingDelivery::new(false));
        let center = NotificationCenter::new(10).with_delivery(delivery.clone());
        let n = center.deliver("oops", None, Priority::Important).await;
        assert_eq!(n.outcome, DeliveryOutcome::Failed);
    }

    #[tokio::test]
    async fn ring_buffer_evicts_oldest() {
        let center = NotificationCenter::new(3);
        center.deliver("a", None, Priority::Routine).await;
        center.deliver("b", None, Priority::Routine).await;
        center.deliver("c", None, Priority::Routine).await;
        center.deliver("d", None, Priority::Routine).await;
        let recent = center.recent(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].text, "d");
        assert_eq!(recent[1].text, "c");
        assert_eq!(recent[2].text, "b");
    }

    #[tokio::test]
    async fn recent_returns_newest_first() {
        let center = NotificationCenter::new(10);
        center.deliver("first", None, Priority::Routine).await;
        center.deliver("second", None, Priority::Routine).await;
        let recent = center.recent(2);
        assert_eq!(recent[0].text, "second");
        assert_eq!(recent[1].text, "first");
    }

    #[tokio::test]
    async fn id_has_expected_shape() {
        let center = NotificationCenter::new(10);
        let n = center.deliver("x", None, Priority::Routine).await;
        // e.g. 1750000000000-a1b2c3d4
        let parts: Vec<&str> = n.id.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].len(), 8);
    }

    #[test]
    #[should_panic(expected = "NotificationCenter capacity must be > 0")]
    fn new_panics_when_capacity_zero() {
        let _ = NotificationCenter::new(0);
    }
    #[derive(Default)]
    struct Kept(Mutex<Vec<Notification>>);

    impl NotificationHistory for Kept {
        fn keep(&self, notification: &Notification) {
            self.0.lock().unwrap().push(notification.clone());
        }
    }

    #[tokio::test]
    async fn what_was_said_before_a_restart_is_still_recalled() {
        let kept = Arc::new(Kept::default());
        let center = NotificationCenter::new(10).with_history(Vec::new(), kept.clone());
        center.deliver("first", None, Priority::Routine).await;
        center.deliver("second", None, Priority::Routine).await;

        // The next run reads them back newest first, as the store does.
        let earlier: Vec<Notification> = kept.0.lock().unwrap().iter().rev().cloned().collect();
        let restarted =
            NotificationCenter::new(10).with_history(earlier, Arc::new(Kept::default()));
        let recent = restarted.recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "second");
        assert_eq!(recent[1].text, "first");
    }

    #[test]
    fn what_is_picked_up_respects_capacity() {
        let earlier: Vec<Notification> = (1..=5)
            .rev()
            .map(|i| Notification {
                id: format!("id-{i}"),
                text: format!("msg {i}"),
                priority: Priority::Routine,
                room: None,
                outcome: DeliveryOutcome::Delivered,
                created_at: Utc::now() + chrono::Duration::seconds(i),
            })
            .collect();
        let center = NotificationCenter::new(3).with_history(earlier, Arc::new(Kept::default()));
        let recent = center.recent(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].text, "msg 5");
        assert_eq!(recent[1].text, "msg 4");
        assert_eq!(recent[2].text, "msg 3");
    }
}
