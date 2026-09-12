//! Recent log lines, kept in memory and served over HTTP.
//!
//! Niles logs to stdout, which on a cluster means the only way to read
//! them is `kubectl logs` — a different tool, different credentials,
//! and in practice someone else's job. That is a poor place to put the
//! answer to "why did it do that", which is a question asked far more
//! often than any other.
//!
//! So the last few hundred lines are also kept here. Not a log
//! *system*: no search, no retention, nothing survives a restart. It
//! answers "what just happened", which is the question that matters
//! when something just happened.
//!
//! # What this exposes
//!
//! Voice transcripts, among other things — what people said in their
//! home. That is a step up in sensitivity from the config this API
//! already serves, and the API has no authentication by design. It is
//! reachable only on the internal network, which is the same
//! assumption every other route here makes, but it is worth knowing
//! rather than discovering.

use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// How many lines to keep. A few hundred covers the last several
/// minutes of a chatty run, which is the window anyone is asking
/// about; keeping more would trade memory for a question nobody is
/// asking of a process that has no retention policy anyway.
const CAPACITY: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub at: String,
    pub level: String,
    /// The module that emitted it — `niles::curve`, `niles_mqtt::source`.
    pub target: String,
    pub message: String,
}

/// A bounded ring of recent log lines, shared between the tracing
/// layer that fills it and the route that serves it.
#[derive(Clone, Default)]
pub struct LogBuffer {
    lines: Arc<Mutex<VecDeque<LogLine>>>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            lines: Arc::new(Mutex::new(VecDeque::with_capacity(CAPACITY))),
        }
    }

    fn push(&self, line: LogLine) {
        let mut guard = self.lock();
        if guard.len() == CAPACITY {
            guard.pop_front();
        }
        guard.push_back(line);
    }

    /// The most recent `tail` lines at or above `min_level`, oldest
    /// first — the order they happened, which is the order they are
    /// read in.
    pub fn recent(&self, tail: usize, min_level: Option<Level>) -> Vec<LogLine> {
        let guard = self.lock();
        let mut out: Vec<LogLine> = guard
            .iter()
            .filter(|l| match min_level {
                Some(min) => parse_level(&l.level).is_some_and(|lvl| lvl <= min),
                None => true,
            })
            .rev()
            .take(tail)
            .cloned()
            .collect();
        out.reverse();
        out
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<LogLine>> {
        self.lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// `tracing` layer that copies each event into the buffer.
///
/// Deliberately a layer rather than a writer: a writer would hand us
/// formatted bytes to parse back apart, and the level and target are
/// worth keeping as fields.
pub struct LogLayer {
    buffer: LogBuffer,
}

impl LogLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut message = MessageVisitor::default();
        event.record(&mut message);
        self.buffer.push(LogLine {
            at: chrono::Utc::now().to_rfc3339(),
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message: message.finish(),
        });
    }
}

/// Collects an event's fields into one line.
///
/// `message` leads, because that is what a person reads; everything
/// else follows as `key=value`, which is how the console layer renders
/// it too.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl MessageVisitor {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            return self.message;
        }
        if self.message.is_empty() {
            return self.fields.join(" ");
        }
        format!("{} {}", self.message, self.fields.join(" "))
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
            // Debug on a string literal quotes it; the console layer
            // shows the message unquoted and so should this.
            if self.message.len() >= 2
                && self.message.starts_with('"')
                && self.message.ends_with('"')
            {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
        } else {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

fn parse_level(raw: &str) -> Option<Level> {
    match raw.to_ascii_uppercase().as_str() {
        "ERROR" => Some(Level::ERROR),
        "WARN" => Some(Level::WARN),
        "INFO" => Some(Level::INFO),
        "DEBUG" => Some(Level::DEBUG),
        "TRACE" => Some(Level::TRACE),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    /// How many lines, newest last. Capped at what is kept.
    #[serde(default)]
    tail: Option<usize>,
    /// `error`, `warn`, `info`, `debug`, `trace` — this level and above.
    #[serde(default)]
    level: Option<String>,
}

/// `GET /logs?tail=100&level=warn`
pub async fn get_logs(
    State(state): State<crate::state::AppState>,
    Query(q): Query<LogQuery>,
) -> Response {
    let Some(buffer) = state.logs.as_ref() else {
        return (
            axum::http::StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({
                "error": "this Niles instance was started without a log buffer"
            })),
        )
            .into_response();
    };

    let min_level = match q.level.as_deref() {
        None => None,
        Some(raw) => match parse_level(raw) {
            Some(level) => Some(level),
            None => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("unknown level {raw:?}; expected error, warn, info, debug or trace")
                    })),
                )
                    .into_response();
            }
        },
    };

    let tail = q.tail.unwrap_or(100).min(CAPACITY);
    Json(buffer.recent(tail, min_level)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: &str, message: &str) -> LogLine {
        LogLine {
            at: "2026-09-12T19:00:00Z".into(),
            level: level.into(),
            target: "niles".into(),
            message: message.into(),
        }
    }

    #[test]
    fn the_newest_lines_are_kept_and_read_in_order() {
        let buf = LogBuffer::new();
        for i in 0..5 {
            buf.push(line("INFO", &format!("line {i}")));
        }
        let recent = buf.recent(3, None);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].message, "line 2", "oldest of the tail first");
        assert_eq!(recent[2].message, "line 4", "newest last");
    }

    #[test]
    fn the_oldest_lines_fall_off_rather_than_growing_forever() {
        let buf = LogBuffer::new();
        for i in 0..(CAPACITY + 10) {
            buf.push(line("INFO", &format!("line {i}")));
        }
        let all = buf.recent(CAPACITY * 2, None);
        assert_eq!(all.len(), CAPACITY);
        assert_eq!(all[0].message, "line 10");
    }

    #[test]
    fn a_level_filter_keeps_that_level_and_worse() {
        // "warn" means warnings and errors — the things you ask for
        // when something went wrong, not a slice of exactly one level.
        let buf = LogBuffer::new();
        buf.push(line("INFO", "fine"));
        buf.push(line("WARN", "hmm"));
        buf.push(line("ERROR", "bad"));

        let warned = buf.recent(10, Some(Level::WARN));
        let messages: Vec<&str> = warned.iter().map(|l| l.message.as_str()).collect();
        assert_eq!(messages, vec!["hmm", "bad"]);
    }

    #[test]
    fn an_empty_buffer_is_not_an_error() {
        assert!(LogBuffer::new().recent(10, None).is_empty());
    }
}
