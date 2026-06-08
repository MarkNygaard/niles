//! Notification center: in-memory ring buffer + delivery orchestration.

// Error/Result unused in this module — kept for future expansion.
use crate::log::NotificationLog;
use crate::model::{DeliveryOutcome, Notification, Priority};
use crate::quiet::QuietHoursConfig;
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Trait for concrete delivery backends (Wyoming, push, etc.).
pub trait NotificationDelivery: Send + Sync {
    /// Attempt to speak/display the notification. Returns true on success.
    fn deliver(&self, text: &str, room: Option<&str>, priority: Priority) -> bool;
}

pub struct NotificationCenter {
    buffer: Mutex<VecDeque<Notification>>,
    capacity: usize,
    quiet: Option<QuietHoursConfig>,
    delivery: Option<Arc<dyn NotificationDelivery>>,
    log: Option<NotificationLog>,
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
            log: None,
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

    /// Attach a file-based notification log.
    ///
    /// Seeds the in-memory ring buffer with the most recent persisted
    /// notifications so `recent()` is immediately useful after restart.
    pub fn with_log(mut self, log: NotificationLog) -> Self {
        match log.load_recent(self.capacity) {
            Ok(recent) => {
                let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
                for n in recent.into_iter().rev() {
                    buffer.push_back(n);
                }
            }
            Err(e) => {
                tracing::warn!("failed to seed notification buffer from log: {e}");
            }
        }
        self.log = Some(log);
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
    pub fn deliver(
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
                    (priority, self.try_deliver(&text, room.as_deref(), priority))
                }
            } else {
                (priority, self.try_deliver(&text, room.as_deref(), priority))
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
        if let Some(ref log) = self.log
            && let Err(e) = log.append(&notification)
        {
            tracing::warn!("failed to persist notification to log: {e}");
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

    fn try_deliver(&self, text: &str, room: Option<&str>, priority: Priority) -> DeliveryOutcome {
        match &self.delivery {
            Some(d) => {
                if d.deliver(text, room, priority) {
                    DeliveryOutcome::Delivered
                } else {
                    DeliveryOutcome::Failed
                }
            }
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

    impl NotificationDelivery for CountingDelivery {
        fn deliver(&self, _text: &str, _room: Option<&str>, _priority: Priority) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.succeed
        }
    }

    fn dummy_quiet(start_h: u32, end_h: u32) -> QuietHoursConfig {
        QuietHoursConfig {
            enabled: true,
            window: Some(crate::quiet::QuietWindow::new(
                chrono::NaiveTime::from_hms_opt(start_h, 0, 0).unwrap(),
                chrono::NaiveTime::from_hms_opt(end_h, 0, 0).unwrap(),
            )),
            timezone: Some(chrono_tz::Tz::UTC),
        }
    }

    #[test]
    fn delivers_routine_when_not_quiet() {
        let delivery = Arc::new(CountingDelivery::new(true));
        let center = NotificationCenter::new(10).with_delivery(delivery.clone());
        let n = center.deliver("hello", None, Priority::Routine);
        assert_eq!(n.outcome, DeliveryOutcome::Delivered);
        assert_eq!(n.priority, Priority::Routine);
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn suppresses_routine_during_quiet_hours() {
        let delivery = Arc::new(CountingDelivery::new(true));
        let center = NotificationCenter::new(10)
            .with_delivery(delivery.clone())
            .with_quiet_hours(dummy_quiet(0, 23));
        let n = center.deliver("shh", None, Priority::Routine);
        assert_eq!(n.outcome, DeliveryOutcome::Suppressed);
        assert_eq!(n.priority, Priority::Important);
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn delivers_important_during_quiet_hours() {
        let delivery = Arc::new(CountingDelivery::new(true));
        let center = NotificationCenter::new(10)
            .with_delivery(delivery.clone())
            .with_quiet_hours(dummy_quiet(0, 23));
        let n = center.deliver("hey", None, Priority::Important);
        assert_eq!(n.outcome, DeliveryOutcome::Delivered);
        assert_eq!(n.priority, Priority::Important);
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn delivers_urgent_unconditionally() {
        let delivery = Arc::new(CountingDelivery::new(true));
        let center = NotificationCenter::new(10)
            .with_delivery(delivery.clone())
            .with_quiet_hours(dummy_quiet(0, 23));
        let n = center.deliver("fire", None, Priority::Urgent);
        assert_eq!(n.outcome, DeliveryOutcome::Delivered);
        assert_eq!(n.priority, Priority::Urgent);
        assert_eq!(delivery.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn records_failed_delivery() {
        let delivery = Arc::new(CountingDelivery::new(false));
        let center = NotificationCenter::new(10).with_delivery(delivery.clone());
        let n = center.deliver("oops", None, Priority::Important);
        assert_eq!(n.outcome, DeliveryOutcome::Failed);
    }

    #[test]
    fn ring_buffer_evicts_oldest() {
        let center = NotificationCenter::new(3);
        center.deliver("a", None, Priority::Routine);
        center.deliver("b", None, Priority::Routine);
        center.deliver("c", None, Priority::Routine);
        center.deliver("d", None, Priority::Routine);
        let recent = center.recent(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].text, "d");
        assert_eq!(recent[1].text, "c");
        assert_eq!(recent[2].text, "b");
    }

    #[test]
    fn recent_returns_newest_first() {
        let center = NotificationCenter::new(10);
        center.deliver("first", None, Priority::Routine);
        center.deliver("second", None, Priority::Routine);
        let recent = center.recent(2);
        assert_eq!(recent[0].text, "second");
        assert_eq!(recent[1].text, "first");
    }

    #[test]
    fn id_has_expected_shape() {
        let center = NotificationCenter::new(10);
        let n = center.deliver("x", None, Priority::Routine);
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
    #[test]
    fn persists_and_reloads_across_restart() {
        use crate::model::Priority;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let log = NotificationLog::new(tmp.path()).unwrap();
        let center = NotificationCenter::new(10).with_log(log);
        center.deliver("first", None, Priority::Routine);
        center.deliver("second", None, Priority::Routine);
        assert_eq!(center.recent(10).len(), 2);

        // Simulate restart: new center seeded from same log.
        let log2 = NotificationLog::new(tmp.path()).unwrap();
        let center2 = NotificationCenter::new(10).with_log(log2);
        let recent = center2.recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "second");
        assert_eq!(recent[1].text, "first");
    }

    #[test]
    fn seed_respects_capacity() {
        use crate::model::{DeliveryOutcome, Priority};
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let log = NotificationLog::new(tmp.path()).unwrap();
        // Write 5 notifications directly to log.
        for i in 1..=5 {
            let n = crate::model::Notification {
                id: format!("id-{i}"),
                text: format!("msg {i}"),
                priority: Priority::Routine,
                room: None,
                outcome: DeliveryOutcome::Delivered,
                created_at: chrono::Utc::now() + chrono::Duration::seconds(i),
            };
            log.append(&n).unwrap();
        }
        // New center with capacity 3 should only keep the newest 3.
        let center = NotificationCenter::new(3).with_log(log);
        let recent = center.recent(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].text, "msg 5");
        assert_eq!(recent[1].text, "msg 4");
        assert_eq!(recent[2].text, "msg 3");
    }
}
