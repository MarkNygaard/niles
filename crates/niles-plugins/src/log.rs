//! A plugin's log as its panel shows it: the last [`KEPT`] lines, from
//! the `log` import, its stdout and stderr, and what the runtime itself
//! has to say about it (started, not set up, failing, stopped).
//!
//! Kept by the runtime per plugin id rather than per instance, so the
//! lines that explain a crash are still there after the restart it
//! caused, and after an update or a rollback replaced the version that
//! wrote them. Memory only: a Niles restart starts every log empty,
//! which is also what keeps a chatty plugin from filling the database.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;

/// Lines kept per plugin; past it the oldest go.
pub const KEPT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

/// One line of a plugin's log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    pub at: DateTime<Utc>,
    pub level: Level,
    pub message: String,
    /// Said by Niles about the plugin, not by the plugin.
    pub niles: bool,
}

/// One plugin's last [`KEPT`] lines.
#[derive(Debug, Default)]
pub struct PluginLog {
    lines: Mutex<VecDeque<Line>>,
}

impl PluginLog {
    pub fn push(&self, level: Level, message: &str, niles: bool) {
        let mut lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        if lines.len() >= KEPT {
            lines.pop_front();
        }
        lines.push_back(Line {
            at: Utc::now(),
            level,
            message: message.to_string(),
            niles,
        });
    }

    /// Every kept line, oldest first.
    pub fn lines(&self) -> Vec<Line> {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

impl From<crate::bindings::niles::integration::log::Level> for Level {
    fn from(level: crate::bindings::niles::integration::log::Level) -> Self {
        use crate::bindings::niles::integration::log::Level as Wit;
        match level {
            Wit::Debug => Level::Debug,
            Wit::Info => Level::Info,
            Wit::Warn => Level::Warn,
            Wit::Error => Level::Error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_thousand_lines_are_kept_oldest_first() {
        let log = PluginLog::default();
        for n in 0..KEPT + 5 {
            log.push(Level::Info, &format!("line {n}"), false);
        }
        let lines = log.lines();
        assert_eq!(lines.len(), KEPT);
        assert_eq!(lines[0].message, "line 5", "the oldest five went");
        assert_eq!(lines[KEPT - 1].message, format!("line {}", KEPT + 4));
    }

    #[test]
    fn a_line_says_who_said_it_and_how_loudly() {
        let log = PluginLog::default();
        log.push(Level::Warn, "rate limited", false);
        log.push(Level::Error, "Not working: trapped in on-timer", true);
        let json = serde_json::to_value(log.lines()).unwrap();
        assert_eq!(json[0]["level"], "warn");
        assert_eq!(json[0]["niles"], false);
        assert_eq!(json[1]["level"], "error");
        assert_eq!(json[1]["niles"], true);
        assert!(json[1]["at"].as_str().unwrap().ends_with('Z'));
    }
}
