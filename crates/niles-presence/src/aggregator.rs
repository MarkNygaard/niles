//! Home-state aggregator with arrive-fast / leave-slow hysteresis.

use crate::state::{HomeState, Override, PresenceSignal, PresenceSnapshot, SourceReading};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Mutex;

/// Aggregates readings from one or more `PresenceSource`s and applies
/// hysteresis so that arriving home is detected immediately while
/// leaving requires a sustained absence.
pub struct PresenceAggregator {
    signals: Mutex<HashMap<String, PresenceSignal>>,
    override_state: Mutex<Override>,
    away_debounce: chrono::Duration,
    away_pending_since: Mutex<Option<DateTime<Utc>>>,
    last_resolved: Mutex<HomeState>,
}

impl PresenceAggregator {
    pub fn new(away_debounce: chrono::Duration) -> Self {
        Self {
            signals: Mutex::new(HashMap::new()),
            override_state: Mutex::new(Override::Auto),
            away_debounce,
            away_pending_since: Mutex::new(None),
            last_resolved: Mutex::new(HomeState::Unknown),
        }
    }

    /// Ingest a new signal, overwriting any prior reading from the
    /// same source.
    pub fn ingest(&self, signal: PresenceSignal) {
        if signal.anyone_home {
            *self.last_resolved.lock().unwrap() = HomeState::Home;
            *self.away_pending_since.lock().unwrap() = None;
        }
        self.signals
            .lock()
            .unwrap()
            .insert(signal.source.clone(), signal);
    }

    /// Set the manual override.
    pub fn set_override(&self, ov: Override) {
        let mut pending = self.away_pending_since.lock().unwrap();
        if ov == Override::Auto {
            *pending = None;
        }
        drop(pending);
        *self.override_state.lock().unwrap() = ov;
    }

    /// Resolve the current household state given `now`.
    pub fn resolve(&self, now: DateTime<Utc>) -> HomeState {
        let ov = *self.override_state.lock().unwrap();

        if ov == Override::ForceHome {
            *self.last_resolved.lock().unwrap() = HomeState::Home;
            return HomeState::Home;
        }
        if ov == Override::ForceAway {
            *self.last_resolved.lock().unwrap() = HomeState::Away;
            return HomeState::Away;
        }

        let signals = self.signals.lock().unwrap();
        if signals.is_empty() {
            drop(signals);
            *self.last_resolved.lock().unwrap() = HomeState::Unknown;
            return HomeState::Unknown;
        }

        if signals.values().any(|s| s.anyone_home) {
            drop(signals);
            *self.away_pending_since.lock().unwrap() = None;
            *self.last_resolved.lock().unwrap() = HomeState::Home;
            return HomeState::Home;
        }

        drop(signals);

        let mut pending = self.away_pending_since.lock().unwrap();
        if pending.is_none() {
            *pending = Some(now);
        }
        let elapsed = now - pending.unwrap();
        drop(pending);

        if elapsed >= self.away_debounce {
            *self.last_resolved.lock().unwrap() = HomeState::Away;
            HomeState::Away
        } else {
            *self.last_resolved.lock().unwrap()
        }
    }

    /// Produce a full snapshot of current state.
    pub fn snapshot(&self, now: DateTime<Utc>) -> PresenceSnapshot {
        let state = self.resolve(now);
        let sources: Vec<SourceReading> = self
            .signals
            .lock()
            .unwrap()
            .values()
            .map(|s| SourceReading {
                source: s.source.clone(),
                anyone_home: s.anyone_home,
                observed_at: s.observed_at,
            })
            .collect();
        let r#override = *self.override_state.lock().unwrap();
        PresenceSnapshot {
            state,
            r#override,
            sources,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn agg() -> PresenceAggregator {
        PresenceAggregator::new(chrono::Duration::minutes(5))
    }

    fn signal(source: &str, anyone_home: bool, at: DateTime<Utc>) -> PresenceSignal {
        PresenceSignal {
            source: source.into(),
            anyone_home,
            observed_at: at,
        }
    }

    #[test]
    fn unknown_when_no_signals() {
        let a = agg();
        assert_eq!(a.resolve(t0()), HomeState::Unknown);
    }

    #[test]
    fn home_when_any_signal_reports_home() {
        let a = agg();
        a.ingest(signal("a", true, t0()));
        assert_eq!(a.resolve(t0()), HomeState::Home);
    }

    #[test]
    fn stays_home_within_debounce_after_blip() {
        let a = agg();
        a.ingest(signal("a", true, t0()));
        let t1 = t0() + chrono::Duration::seconds(10);
        a.ingest(signal("a", false, t1));
        assert_eq!(a.resolve(t1), HomeState::Home);
        assert_eq!(
            a.resolve(t0() + chrono::Duration::minutes(4)),
            HomeState::Home
        );
    }

    #[test]
    fn flips_to_away_after_debounce_elapses() {
        let a = agg();
        a.ingest(signal("a", true, t0()));
        let t1 = t0() + chrono::Duration::seconds(10);
        a.ingest(signal("a", false, t1));
        assert_eq!(a.resolve(t1), HomeState::Home);
        assert_eq!(
            a.resolve(t1 + chrono::Duration::minutes(4)),
            HomeState::Home
        );
        assert_eq!(
            a.resolve(t1 + chrono::Duration::minutes(5) + chrono::Duration::seconds(1)),
            HomeState::Away
        );
    }

    #[test]
    fn away_pending_resets_on_new_home_signal() {
        let a = agg();
        let t0 = t0();
        a.ingest(signal("a", true, t0));
        let t1 = t0 + chrono::Duration::seconds(10);
        a.ingest(signal("a", false, t1));
        let t2 = t0 + chrono::Duration::minutes(2);
        assert_eq!(a.resolve(t2), HomeState::Home); // still within debounce

        let t3 = t0 + chrono::Duration::minutes(3);
        a.ingest(signal("a", true, t3));
        assert_eq!(a.resolve(t3), HomeState::Home);

        let t4 = t0 + chrono::Duration::minutes(9);
        assert_eq!(a.resolve(t4), HomeState::Home); // true signal still stuck

        let t5 = t0 + chrono::Duration::minutes(9);
        a.ingest(signal("a", false, t5));
        let t6 = t0 + chrono::Duration::minutes(14);
        assert_eq!(a.resolve(t6), HomeState::Home); // within debounce of latest pending start

        assert_eq!(
            a.resolve(t6 + chrono::Duration::minutes(5) + chrono::Duration::seconds(1)),
            HomeState::Away
        );
    }

    #[test]
    fn force_away_overrides_home_signal() {
        let a = agg();
        a.ingest(signal("a", true, t0()));
        a.set_override(Override::ForceAway);
        assert_eq!(a.resolve(t0()), HomeState::Away);
    }

    #[test]
    fn force_home_overrides_all_away_signals() {
        let a = agg();
        a.ingest(signal("a", false, t0()));
        a.set_override(Override::ForceHome);
        assert_eq!(a.resolve(t0()), HomeState::Home);
    }

    #[test]
    fn auto_resumes_signals() {
        let a = agg();
        a.ingest(signal("a", true, t0()));
        a.set_override(Override::ForceAway);
        assert_eq!(a.resolve(t0()), HomeState::Away);
        a.set_override(Override::Auto);
        assert_eq!(a.resolve(t0()), HomeState::Home);
    }

    #[test]
    fn two_sources_any_home_wins() {
        let a = agg();
        a.ingest(signal("a", true, t0()));
        a.ingest(signal("b", false, t0()));
        assert_eq!(a.resolve(t0()), HomeState::Home);
    }

    #[test]
    fn snapshot_includes_per_source_readings_and_override() {
        let a = agg();
        a.ingest(signal("a", true, t0()));
        let snap = a.snapshot(t0());
        assert_eq!(snap.state, HomeState::Home);
        assert_eq!(snap.r#override, Override::Auto);
        assert_eq!(snap.sources.len(), 1);
        assert_eq!(snap.sources[0].source, "a");
    }
}
