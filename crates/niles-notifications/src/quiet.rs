//! Quiet-hours configuration and window evaluation.

use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;

/// A daily time window when Routine notifications are suppressed.
#[derive(Debug, Clone)]
pub struct QuietWindow {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

impl QuietWindow {
    /// Create a new quiet window.
    pub fn new(start: NaiveTime, end: NaiveTime) -> Self {
        Self { start, end }
    }

    /// Returns true if `timestamp` falls inside this quiet window.
    ///
    /// Handles both non-wrapping (10:00–14:00) and wrapping
    /// (22:00–07:00) windows, including exact boundary times.
    pub fn covers(&self, timestamp: DateTime<Utc>, tz: Tz) -> bool {
        let local = timestamp.with_timezone(&tz);
        let t = local.time();

        if self.start <= self.end {
            // Non-wrapping: e.g. 10:00–14:00
            t >= self.start && t <= self.end
        } else {
            // Wrapping: e.g. 22:00–07:00
            t >= self.start || t <= self.end
        }
    }
}

/// Full quiet-hours configuration for the household.
#[derive(Debug, Clone, Default)]
pub struct QuietHoursConfig {
    pub enabled: bool,
    pub window: Option<QuietWindow>,
    pub timezone: Option<Tz>,
}

impl QuietHoursConfig {
    /// Returns true if quiet hours are currently active.
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(window) = &self.window else {
            return false;
        };
        let tz = self.timezone.unwrap_or(Tz::UTC);
        window.covers(now, tz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc_time(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
    }

    fn utc_dt(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn non_wrapping_window_covers_inside() {
        let w = QuietWindow::new(utc_time(10, 0), utc_time(14, 0));
        assert!(w.covers(utc_dt(2026, 1, 1, 12, 0), Tz::UTC));
    }

    #[test]
    fn non_wrapping_window_excludes_outside() {
        let w = QuietWindow::new(utc_time(10, 0), utc_time(14, 0));
        assert!(!w.covers(utc_dt(2026, 1, 1, 9, 0), Tz::UTC));
        assert!(!w.covers(utc_dt(2026, 1, 1, 15, 0), Tz::UTC));
    }

    #[test]
    fn non_wrapping_includes_boundaries() {
        let w = QuietWindow::new(utc_time(10, 0), utc_time(14, 0));
        assert!(w.covers(utc_dt(2026, 1, 1, 10, 0), Tz::UTC));
        assert!(w.covers(utc_dt(2026, 1, 1, 14, 0), Tz::UTC));
    }

    #[test]
    fn wrapping_window_covers_inside() {
        let w = QuietWindow::new(utc_time(22, 0), utc_time(7, 0));
        assert!(w.covers(utc_dt(2026, 1, 1, 23, 0), Tz::UTC));
        assert!(w.covers(utc_dt(2026, 1, 2, 3, 0), Tz::UTC));
    }

    #[test]
    fn wrapping_window_excludes_outside() {
        let w = QuietWindow::new(utc_time(22, 0), utc_time(7, 0));
        assert!(!w.covers(utc_dt(2026, 1, 1, 12, 0), Tz::UTC));
        assert!(!w.covers(utc_dt(2026, 1, 2, 8, 0), Tz::UTC));
    }

    #[test]
    fn wrapping_includes_boundaries() {
        let w = QuietWindow::new(utc_time(22, 0), utc_time(7, 0));
        assert!(w.covers(utc_dt(2026, 1, 1, 22, 0), Tz::UTC));
        assert!(w.covers(utc_dt(2026, 1, 2, 7, 0), Tz::UTC));
    }

    #[test]
    fn config_disabled_never_active() {
        let cfg = QuietHoursConfig {
            enabled: false,
            window: Some(QuietWindow::new(utc_time(22, 0), utc_time(7, 0))),
            timezone: Some(Tz::UTC),
        };
        assert!(!cfg.is_active(utc_dt(2026, 1, 1, 23, 0)));
    }

    #[test]
    fn config_no_window_never_active() {
        let cfg = QuietHoursConfig {
            enabled: true,
            window: None,
            timezone: Some(Tz::UTC),
        };
        assert!(!cfg.is_active(utc_dt(2026, 1, 1, 23, 0)));
    }

    #[test]
    fn config_with_timezone_conversion() {
        // 22:00 UTC = 23:00 Europe/Copenhagen (winter)
        let w = QuietWindow::new(utc_time(22, 0), utc_time(7, 0));
        let cfg = QuietHoursConfig {
            enabled: true,
            window: Some(w),
            timezone: Some("Europe/Copenhagen".parse().unwrap()),
        };
        // 23:00 UTC → 00:00 CET+1, which is inside 22:00–07:00 local
        assert!(cfg.is_active(utc_dt(2026, 1, 1, 23, 0)));
    }
}
