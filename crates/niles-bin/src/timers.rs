//! Timers: the driver that rings them and how they are named.

use chrono::Utc;
use niles_core::{Event, EventBus};
use niles_scheduler::{TimerEntry, TimerStore};
use std::sync::Arc;

/// Spawn the timer driver task. Wakes on the soonest pending
/// expiry (`TimerStore::next_expiry`), transitions matching
/// timers from `Pending` → `Ringing`, prints a user-visible
/// `[timer] FIRED` line, and publishes [`Event::TimerFired`]
/// for any future consumer (satellite-alarm playback is
/// XVF3800-blocked, out of scope here).
///
/// The sleep is capped at 1 s — without a notify mechanism, a timer
/// added during a long sleep would otherwise miss its deadline (a 10 s
/// timer set while the driver slept could fire tens of seconds late).
/// Re-checking at least once a second means a freshly-added short timer
/// is at worst ~1 s overdue; the idle poll is negligible CPU.
///
/// Shared by `voice_dispatch` and `serve`.
pub(crate) fn spawn_timer_driver(
    timers: Arc<TimerStore>,
    bus: EventBus,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use tokio::time::sleep;
        const MAX_SLEEP: std::time::Duration = std::time::Duration::from_secs(1);
        loop {
            let now = Utc::now();
            let sleep_for = match timers.next_expiry() {
                Some(exp) => {
                    let ms = (exp - now).num_milliseconds().max(0) as u64;
                    std::time::Duration::from_millis(ms).min(MAX_SLEEP)
                }
                None => MAX_SLEEP,
            };
            sleep(sleep_for).await;

            let now = Utc::now();
            for entry in timers
                .list()
                .into_iter()
                .filter(|e| e.is_pending() && e.expires_at <= now)
            {
                if let Some(rung) = timers.mark_ringing(entry.id) {
                    println!(
                        "[timer] FIRED {} (peer={}, id={})",
                        timer_label(&rung),
                        rung.origin,
                        rung.id.0,
                    );
                    bus.publish(Event::TimerFired {
                        id: rung.id.0,
                        name: rung.name.clone(),
                        origin: rung.origin,
                    });
                }
            }
        }
    })
}

/// Presentation helper for a timer entry — lives in the binary, not
/// the scheduler, because user-facing strings are a dispatch concern.
pub(crate) fn timer_label(entry: &TimerEntry) -> String {
    timer_label_for(entry.name.as_deref(), entry.duration)
}

/// Shared shape used both before a timer is stored (`TimerSet` dispatch
/// arm — entry not yet built) and after (`stop`/`list`/`fired`).
pub(crate) fn timer_label_for(name: Option<&str>, duration: std::time::Duration) -> String {
    if let Some(n) = name {
        return format!("'{n}' timer");
    }
    let secs = duration.as_secs();
    if secs >= 3600 {
        format!("{} hour timer", secs / 3600)
    } else if secs >= 60 {
        format!("{} minute timer", secs / 60)
    } else {
        format!("{secs} second timer")
    }
}

#[cfg(test)]
mod timer_label_tests {
    use super::*;

    use std::time::Duration;

    #[test]
    fn named_timer_ignores_duration() {
        assert_eq!(
            timer_label_for(Some("pasta"), Duration::from_secs(30)),
            "'pasta' timer"
        );
    }

    #[test]
    fn sub_minute_durations_render_in_seconds() {
        // Regression: previously `30s / 60 = 0` gave "0 minute timer".
        assert_eq!(
            timer_label_for(None, Duration::from_secs(30)),
            "30 second timer"
        );
    }

    #[test]
    fn minute_range_renders_in_minutes() {
        assert_eq!(
            timer_label_for(None, Duration::from_secs(60)),
            "1 minute timer"
        );
        assert_eq!(
            timer_label_for(None, Duration::from_secs(8 * 60)),
            "8 minute timer"
        );
    }

    #[test]
    fn hour_range_renders_in_hours() {
        assert_eq!(
            timer_label_for(None, Duration::from_secs(3600)),
            "1 hour timer"
        );
        assert_eq!(
            timer_label_for(None, Duration::from_secs(2 * 3600)),
            "2 hour timer"
        );
    }
}

#[cfg(test)]
mod spawn_timer_driver_tests {
    use super::*;
    use std::net::SocketAddr;
    use std::time::Duration;

    fn localhost() -> SocketAddr {
        "127.0.0.1:9999".parse().unwrap()
    }

    #[tokio::test]
    async fn driver_transitions_expired_timer_and_publishes_event() {
        let timers = Arc::new(TimerStore::new());
        let bus = EventBus::default();
        // Subscribe before spawning — broadcast::Receiver only sees
        // events published after subscription.
        let mut rx = bus.subscribe();
        let id = timers.set(Duration::from_secs(0), None, localhost(), Utc::now());

        let _handle = spawn_timer_driver(Arc::clone(&timers), bus.clone());

        // Deterministic: wait for the event with a generous timeout
        // instead of a fixed sleep that races CI load.
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("driver did not publish TimerFired within 2s")
            .expect("event bus closed unexpectedly");
        match event {
            Event::TimerFired {
                id: fired_id,
                name,
                origin,
            } => {
                assert_eq!(fired_id, id.0);
                assert_eq!(name, None);
                assert_eq!(origin, localhost());
            }
            other => panic!("expected TimerFired, got {other:?}"),
        }

        let entries = timers.list();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].is_ringing(),
            "expected timer {id:?} to be Ringing, got {:?}",
            entries[0].state
        );
    }
}
