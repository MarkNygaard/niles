//! Command history types, writer, reader, and query filter.

use crate::error::{Error, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::PathBuf;

/// One voice-command turn, stored as a single JSONL line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandEntry {
    /// UTC timestamp of the turn.
    pub ts: DateTime<Utc>,
    /// Satellite peer that sent the audio.
    pub peer: SocketAddr,
    /// Canonical room of the satellite, if known.
    #[serde(default)]
    pub origin_room: Option<String>,
    /// Transcribed user utterance.
    pub transcript: String,
    /// Spoken response returned to the user (None = no response / error).
    #[serde(default)]
    pub spoken_response: Option<String>,
}

/// Filter for [`CommandReader::query`].
#[derive(Debug, Clone, Default)]
pub struct CommandQuery {
    /// Inclusive lower bound on `ts`.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive upper bound on `ts`.
    pub until: Option<DateTime<Utc>>,
    /// Match `origin_room` exactly.
    pub room: Option<String>,
    /// Max rows to return (default 50, clamped to 500).
    pub limit: Option<usize>,
}

/// Append-only JSONL writer, partitioned by UTC date.
pub struct CommandWriter {
    root: PathBuf,
    enabled: bool,
}

impl CommandWriter {
    /// Create a writer that persists under `root/commands/`.
    /// Creates the subdirectory if it does not exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let dir = root.join("commands");
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            root,
            enabled: true,
        })
    }

    /// No-op writer that never touches the filesystem.
    pub fn disabled() -> Self {
        Self {
            root: PathBuf::new(),
            enabled: false,
        }
    }

    /// Append `entry` to today's JSONL file.
    pub fn append(&self, entry: &CommandEntry) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let date = entry.ts.date_naive();
        let path = self
            .root
            .join("commands")
            .join(format!("{}.jsonl", date.format("%Y-%m-%d")));
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        serde_json::to_writer(&mut file, entry)?;
        file.write_all(b"\n")?;
        Ok(())
    }

    /// Remove JSONL files older than `retention_days`.
    pub fn prune(&self, retention_days: u32) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let cutoff = Utc::now().date_naive() - chrono::Duration::days(retention_days as i64);
        let commands_dir = self.root.join("commands");
        let entries = match std::fs::read_dir(&commands_dir) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let date = match NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => {
                    tracing::debug!("skipping non-date file in history dir: {}", path.display());
                    continue;
                }
            };
            if date < cutoff {
                match std::fs::remove_file(&path) {
                    Ok(()) => tracing::info!("pruned old history file: {}", path.display()),
                    Err(e) => tracing::warn!("failed to prune {}: {e}", path.display()),
                }
            }
        }
        Ok(())
    }
}

/// Read-side of the command history, filterable and newest-first.
pub struct CommandReader {
    root: PathBuf,
}

impl CommandReader {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Query history entries matching `filter`, returned newest-first.
    pub fn query(&self, filter: &CommandQuery) -> Result<Vec<CommandEntry>> {
        let limit = filter.limit.unwrap_or(50).min(500);

        if let (Some(since), Some(until)) = (filter.since, filter.until)
            && since > until
        {
            return Err(Error::InvalidDateRange {
                reason: "since > until".into(),
            });
        }

        let start_date = filter.since.map(|t| t.date_naive());
        let end_date = filter
            .until
            .map(|t| t.date_naive())
            .unwrap_or_else(|| Utc::now().date_naive());

        let commands_dir = self.root.join("commands");
        let dir_entries = match std::fs::read_dir(&commands_dir) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut results = Vec::new();

        for entry in dir_entries {
            let entry = entry?;
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let date = match NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => continue,
            };
            if start_date.is_some_and(|s| date < s) || date > end_date {
                continue;
            }

            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<CommandEntry>(&line) {
                    Ok(e) => results.push(e),
                    Err(e) => {
                        tracing::warn!("skip malformed history line in {}: {e}", path.display());
                    }
                }
            }
        }

        // Post-filter on exact timestamps / room
        results.retain(|e| {
            if let Some(since) = filter.since
                && e.ts < since
            {
                return false;
            }
            if let Some(until) = filter.until
                && e.ts > until
            {
                return false;
            }
            if let Some(ref room) = filter.room {
                return e.origin_room.as_deref() == Some(room);
            }
            true
        });

        results.sort_by_key(|b| std::cmp::Reverse(b.ts));
        results.truncate(limit);
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn make_entry(ts: DateTime<Utc>, transcript: &str, room: Option<&str>) -> CommandEntry {
        CommandEntry {
            ts,
            peer: "127.0.0.1:1234".parse().unwrap(),
            origin_room: room.map(|s| s.to_string()),
            transcript: transcript.to_string(),
            spoken_response: None,
        }
    }

    #[test]
    fn round_trip() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();
        let reader = CommandReader::new(tmp.path());

        let now = Utc::now();
        for i in 1..=3 {
            writer
                .append(&make_entry(
                    now + chrono::Duration::seconds(i),
                    &format!("turn {i}"),
                    None,
                ))
                .unwrap();
        }

        let entries = reader.query(&CommandQuery::default()).unwrap();
        assert_eq!(entries.len(), 3);
        // newest-first
        assert_eq!(entries[0].transcript, "turn 3");
        assert_eq!(entries[1].transcript, "turn 2");
        assert_eq!(entries[2].transcript, "turn 1");
    }

    #[test]
    fn daily_rotation() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();

        let day1 = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap();
        writer.append(&make_entry(day1, "day one", None)).unwrap();
        writer.append(&make_entry(day2, "day two", None)).unwrap();

        let commands_dir = tmp.path().join("commands");
        assert!(commands_dir.join("2026-01-01.jsonl").exists());
        assert!(commands_dir.join("2026-01-02.jsonl").exists());

        let reader = CommandReader::new(tmp.path());
        let entries = reader.query(&CommandQuery::default()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].transcript, "day two");
        assert_eq!(entries[1].transcript, "day one");
    }

    #[test]
    fn retention_prune() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();

        let old = Utc::now() - chrono::Duration::days(20);
        let recent = Utc::now() - chrono::Duration::days(1);
        writer.append(&make_entry(old, "old", None)).unwrap();
        writer.append(&make_entry(recent, "recent", None)).unwrap();

        writer.prune(14).unwrap();

        let reader = CommandReader::new(tmp.path());
        let entries = reader.query(&CommandQuery::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].transcript, "recent");
    }

    #[test]
    fn disabled_writer_is_no_op() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::disabled();
        let entry = make_entry(Utc::now(), "x", None);
        writer.append(&entry).unwrap();
        writer.prune(1).unwrap();

        // filesystem untouched
        assert!(!tmp.path().join("commands").exists());
    }

    #[test]
    fn query_by_since_until() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();
        let reader = CommandReader::new(tmp.path());

        let d1 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let d2 = Utc.with_ymd_and_hms(2026, 1, 2, 10, 0, 0).unwrap();
        let d3 = Utc.with_ymd_and_hms(2026, 1, 3, 10, 0, 0).unwrap();
        writer.append(&make_entry(d1, "d1", None)).unwrap();
        writer.append(&make_entry(d2, "d2", None)).unwrap();
        writer.append(&make_entry(d3, "d3", None)).unwrap();

        let q = CommandQuery {
            since: Some(d2),
            until: Some(d3),
            ..Default::default()
        };
        let entries = reader.query(&q).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].transcript, "d3");
        assert_eq!(entries[1].transcript, "d2");
    }

    #[test]
    fn query_by_room() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();
        let reader = CommandReader::new(tmp.path());

        let now = Utc::now();
        writer
            .append(&make_entry(now, "kitchen cmd", Some("kitchen")))
            .unwrap();
        writer
            .append(&make_entry(now, "bedroom cmd", Some("bedroom")))
            .unwrap();

        let q = CommandQuery {
            room: Some("kitchen".into()),
            ..Default::default()
        };
        let entries = reader.query(&q).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].transcript, "kitchen cmd");
    }

    #[test]
    fn limit_default() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();
        let reader = CommandReader::new(tmp.path());

        let now = Utc::now();
        for i in 0..60 {
            writer
                .append(&make_entry(
                    now + chrono::Duration::seconds(i),
                    &format!("turn {i}"),
                    None,
                ))
                .unwrap();
        }

        let entries = reader.query(&CommandQuery::default()).unwrap();
        assert_eq!(entries.len(), 50); // default
    }

    #[test]
    fn limit_explicit_and_clamp() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();
        let reader = CommandReader::new(tmp.path());

        let now = Utc::now();
        for i in 0..60 {
            writer
                .append(&make_entry(
                    now + chrono::Duration::seconds(i),
                    &format!("turn {i}"),
                    None,
                ))
                .unwrap();
        }

        let q10 = CommandQuery {
            limit: Some(10),
            ..Default::default()
        };
        assert_eq!(reader.query(&q10).unwrap().len(), 10);

        let q1000 = CommandQuery {
            limit: Some(1000),
            ..Default::default()
        };
        assert_eq!(reader.query(&q1000).unwrap().len(), 60); // clamped to 500, but only 60 exist
    }

    #[test]
    fn newest_first_ordering() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();
        let reader = CommandReader::new(tmp.path());

        let now = Utc::now();
        writer.append(&make_entry(now, "first", None)).unwrap();
        writer
            .append(&make_entry(
                now + chrono::Duration::seconds(5),
                "second",
                None,
            ))
            .unwrap();

        let entries = reader.query(&CommandQuery::default()).unwrap();
        assert_eq!(entries[0].transcript, "second");
        assert_eq!(entries[1].transcript, "first");
    }

    #[test]
    fn corrupted_line_tolerance() {
        let tmp = TempDir::new().unwrap();
        let writer = CommandWriter::new(tmp.path()).unwrap();
        let reader = CommandReader::new(tmp.path());

        let now = Utc::now();
        let t1 = now;
        let t2 = now + chrono::Duration::seconds(1);
        writer.append(&make_entry(t1, "valid1", None)).unwrap();

        // manually append garbage
        let path = tmp
            .path()
            .join("commands")
            .join(format!("{}.jsonl", now.date_naive().format("%Y-%m-%d")));
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"this is not json\n").unwrap();
        }

        writer.append(&make_entry(t2, "valid2", None)).unwrap();

        let entries = reader.query(&CommandQuery::default()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].transcript, "valid2");
        assert_eq!(entries[1].transcript, "valid1");
    }

    #[test]
    fn missing_dir_at_construction() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("nested");
        assert!(!root.exists());
        let writer = CommandWriter::new(&root).unwrap();
        assert!(root.join("commands").exists());
        writer.append(&make_entry(Utc::now(), "x", None)).unwrap();
    }

    #[test]
    fn reader_on_empty_or_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let reader = CommandReader::new(tmp.path());
        let entries = reader.query(&CommandQuery::default()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn since_greater_than_until_errors() {
        let tmp = TempDir::new().unwrap();
        let reader = CommandReader::new(tmp.path());

        let q = CommandQuery {
            since: Some(Utc::now()),
            until: Some(Utc::now() - chrono::Duration::hours(1)),
            ..Default::default()
        };
        let err = reader.query(&q).unwrap_err();
        assert!(matches!(err, Error::InvalidDateRange { .. }));
    }
}
