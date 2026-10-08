//! A word before the groceries arrive.
//!
//! nemlig.com delivers in a window — "between 7 and 9" — and the window
//! is easy to forget by the morning it comes. Half an hour before it
//! opens, the satellites say so, once per order. Routine priority: a
//! delivery at seven is not worth waking anybody for at half past six,
//! and quiet hours decide that, not this.

use chrono::{NaiveDateTime, Timelike};
use niles_config::ConfigStore;
use niles_nemlig::{Credentials, NemligClient, Order};
use niles_notifications::{NotificationCenter, Priority};
use std::sync::Arc;
use std::time::Duration;

/// How often nemlig is asked. A delivery window is hours long; five
/// minutes late on a thirty-minute warning is still a warning.
const EVERY: Duration = Duration::from_secs(5 * 60);

/// How long before the window opens to say so.
const AHEAD_MINUTES: i64 = 30;

pub(crate) fn spawn_reminder(
    client: Arc<NemligClient>,
    config: Arc<ConfigStore>,
    notifications: Arc<NotificationCenter>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut said_for: Option<i64> = None;
        let mut ticker = tokio::time::interval(EVERY);
        loop {
            ticker.tick().await;
            // Read per tick: switched on or off in the app, it follows.
            let Some(credentials) = credentials(&config) else {
                continue;
            };
            let now = niles_nemlig::danish_now();
            let next = match client.next_delivery(&credentials, now).await {
                Ok(next) => next,
                Err(e) => {
                    tracing::debug!("[nemlig] could not look for a delivery: {e}");
                    continue;
                }
            };
            if let Some(order) = next.filter(|o| Some(o.id) != said_for)
                && let Some(text) = reminder(&order, now)
            {
                tracing::info!("[nemlig] {text}");
                notifications.deliver(text, None, Priority::Routine);
                said_for = Some(order.id);
            }
        }
    })
}

fn credentials(config: &ConfigStore) -> Option<Credentials> {
    let cfg = config.current();
    let nemlig = cfg.integrations.nemlig.as_ref().filter(|n| n.enabled)?;
    let (username, password) = nemlig.resolve_credentials().ok()?;
    Some(Credentials { username, password })
}

/// What to say, when it is time to say it: from half an hour before the
/// window opens until it does.
fn reminder(order: &Order, now: NaiveDateTime) -> Option<String> {
    let start = order.delivery_start?;
    let minutes = (start - now).num_minutes();
    if !(0..=AHEAD_MINUTES).contains(&minutes) {
        return None;
    }
    let from = spoken(start);
    Some(match order.delivery_end {
        Some(end) => format!(
            "Your groceries from nemlig arrive between {from} and {}.",
            spoken(end)
        ),
        None => format!("Your groceries from nemlig arrive from {from}."),
    })
}

/// "7", or "7:30" — the way a time is said, not written.
fn spoken(at: NaiveDateTime) -> String {
    match at.minute() {
        0 => at.hour().to_string(),
        m => format!("{}:{m:02}", at.hour()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn order() -> Order {
        Order {
            id: 1,
            status: 1,
            total: Some(512.5),
            delivery_start: Some(at("2026-10-10 07:00")),
            delivery_end: Some(at("2026-10-10 09:00")),
        }
    }

    #[test]
    fn says_it_half_an_hour_before() {
        assert_eq!(
            reminder(&order(), at("2026-10-10 06:35")).as_deref(),
            Some("Your groceries from nemlig arrive between 7 and 9.")
        );
    }

    #[test]
    fn says_nothing_long_before_or_after_it_opens() {
        assert_eq!(reminder(&order(), at("2026-10-10 06:00")), None);
        assert_eq!(reminder(&order(), at("2026-10-10 07:10")), None);
    }

    #[test]
    fn a_half_hour_is_said_as_one() {
        assert_eq!(spoken(at("2026-10-10 17:30")), "17:30");
    }
}
