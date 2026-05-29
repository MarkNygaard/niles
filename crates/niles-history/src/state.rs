//! Device state history types, writer, reader, and query filter.

use crate::error::{Error, Result};
use chrono::{DateTime, NaiveDate, Utc};
use niles_core::{DeviceId, DeviceState};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// One device state snapshot, stored as a single JSONL line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateEntry {
    /// UTC timestamp of the state change.
    pub ts: DateTime<Utc>,
    /// Source-qualified device identifier.
    pub device_id: DeviceId,
    /// Device state at this timestamp.
    pub state: DeviceState,
}

/// Filter for [`StateReader::query`].
#[derive(Debug, Clone, Default)]
pub struct StateQuery {
    /// Inclusive lower bound on `ts`.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive upper bound on `ts`.
    pub until: Option<DateTime<Utc>>,
    /// Match a specific device id exactly.
    pub device_id: Option<DeviceId>,
    /// Match devices in this room (`device_id` wins if both are set).
    pub room: Option<String>,
    /// Max rows to return (default 200, clamped to 2000).
    pub limit: Option<usize>,
}

/// Append-only JSONL writer for device state, partitioned by UTC date.
pub struct StateWriter {
    root: PathBuf,
    enabled: bool,
    lock: std::sync::Mutex<()>,
}

impl StateWriter {
    /// Create a writer that persists under `root/state/`.
    /// Creates the subdirectory if it does not exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let dir = root.join("state");
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            root,
            enabled: true,
            lock: std::sync::Mutex::new(()),
        })
    }

    /// No-op writer that never touches the filesystem.
    pub fn disabled() -> Self {
        Self {
            root: PathBuf::new(),
            enabled: false,
            lock: std::sync::Mutex::new(()),
        }
    }

    /// Append `entry` to the JSONL file for its UTC date.
    pub fn append(&self, entry: &StateEntry) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let _guard = self.lock.lock().unwrap();
        let date = entry.ts.date_naive();
        let path = self
            .root
            .join("state")
            .join(format!("{}.jsonl", date.format("%Y-%m-%d")));
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut buf = serde_json::to_vec(entry)?;
        buf.push(b'\n');
        file.write_all(&buf)?;
        Ok(())
    }

    /// Remove JSONL files older than `retention_days`.
    pub fn prune(&self, retention_days: u32) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let oldest_kept =
            Utc::now().date_naive() - chrono::Duration::days((retention_days as i64) - 1);
        let state_dir = self.root.join("state");
        let entries = match std::fs::read_dir(&state_dir) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let date = match NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => {
                    tracing::debug!(
                        "skipping non-date file in state history dir: {}",
                        path.display()
                    );
                    continue;
                }
            };
            if date < oldest_kept {
                match std::fs::remove_file(&path) {
                    Ok(()) => tracing::info!("pruned old state history file: {}", path.display()),
                    Err(e) => tracing::warn!("failed to prune {}: {e}", path.display()),
                }
            }
        }
        Ok(())
    }
}

/// Read-side of the device state history, filterable and newest-first.
pub struct StateReader {
    root: PathBuf,
    enabled: bool,
}

impl StateReader {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            enabled: true,
        }
    }

    /// No-op reader that always returns empty results.
    pub fn disabled() -> Self {
        Self {
            root: PathBuf::new(),
            enabled: false,
        }
    }

    /// List state JSONL files under `root/state/` whose stem parses as `YYYY-MM-DD`.
    /// Returns `Ok(empty)` when the directory does not exist.
    fn state_files(&self) -> Result<Vec<(NaiveDate, PathBuf)>> {
        let state_dir = self.root.join("state");
        let entries = match std::fs::read_dir(&state_dir) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let date = match NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => continue,
            };
            files.push((date, path));
        }
        Ok(files)
    }

    /// Query state entries matching `filter`, returned newest-first.
    pub fn query(&self, filter: &StateQuery) -> Result<Vec<StateEntry>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let limit = filter.limit.unwrap_or(200).min(2000);

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

        let mut results = Vec::new();

        for (date, path) in self.state_files()? {
            if start_date.is_some_and(|s| date < s) || date > end_date {
                continue;
            }

            let file = match File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<StateEntry>(&line) {
                    Ok(e) => results.push(e),
                    Err(e) => {
                        tracing::warn!(
                            "skip malformed state history line in {}: {e}",
                            path.display()
                        );
                    }
                }
            }
        }

        // Post-filter on exact timestamps / device_id / room
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
            if let Some(ref id) = filter.device_id {
                return e.device_id == *id;
            }
            if let Some(ref room) = filter.room {
                return e.device_id.room().as_str() == room;
            }
            true
        });

        results.sort_by_key(|b| std::cmp::Reverse(b.ts));
        results.truncate(limit);
        Ok(results)
    }

    /// Return the most-recent `StateEntry` per requested device id
    /// whose `ts <= at`. Omits ids with no prior entry.
    pub fn snapshot_at(
        &self,
        at: DateTime<Utc>,
        device_ids: &[DeviceId],
    ) -> Result<Vec<StateEntry>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let target_date = at.date_naive();

        let mut files: Vec<_> = self
            .state_files()?
            .into_iter()
            .filter(|(d, _)| *d <= target_date)
            .collect();
        files.sort_by_key(|(d, _)| *d);

        let id_set: HashSet<_> = device_ids.iter().collect();
        let mut latest: HashMap<DeviceId, StateEntry> = HashMap::new();

        for (_, path) in files {
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<StateEntry>(&line) {
                    Ok(entry) => {
                        if entry.ts <= at && id_set.contains(&entry.device_id) {
                            latest.insert(entry.device_id.clone(), entry);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "skip malformed state history line in {}: {e}",
                            path.display()
                        );
                    }
                }
            }
        }

        let mut results: Vec<StateEntry> = latest.into_values().collect();
        results.sort_by_key(|a| a.device_id.to_string());
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_id(s: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:{s}")).unwrap()
    }

    fn make_entry(ts: DateTime<Utc>, id_str: &str, state: DeviceState) -> StateEntry {
        StateEntry {
            ts,
            device_id: make_id(id_str),
            state,
        }
    }

    #[test]
    fn round_trip() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let now = Utc::now();
        for i in 1..=3 {
            writer
                .append(&make_entry(
                    now + chrono::Duration::seconds(i),
                    "kitchen/ceiling_light",
                    DeviceState {
                        on: Some(true),
                        brightness: Some((i * 10) as u8),
                        ..Default::default()
                    },
                ))
                .unwrap();
        }

        let entries = reader.query(&StateQuery::default()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].state.brightness, Some(30));
        assert_eq!(entries[1].state.brightness, Some(20));
        assert_eq!(entries[2].state.brightness, Some(10));
    }

    #[test]
    fn daily_rotation() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();

        let day1 = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap();
        writer
            .append(&make_entry(
                day1,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                day2,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();

        let state_dir = tmp.path().join("state");
        assert!(state_dir.join("2026-01-01.jsonl").exists());
        assert!(state_dir.join("2026-01-02.jsonl").exists());

        let reader = StateReader::new(tmp.path());
        let entries = reader.query(&StateQuery::default()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].state.on, Some(false));
        assert_eq!(entries[1].state.on, Some(true));
    }

    #[test]
    fn query_by_device_id() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let now = Utc::now();
        writer
            .append(&make_entry(
                now,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                now,
                "living_room/floor_lamp",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                now,
                "bedroom/night_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();

        let q = StateQuery {
            device_id: Some(make_id("living_room/floor_lamp")),
            ..Default::default()
        };
        let entries = reader.query(&q).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, make_id("living_room/floor_lamp"));
    }

    #[test]
    fn query_by_room() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let now = Utc::now();
        writer
            .append(&make_entry(
                now,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                now,
                "kitchen/table_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                now,
                "living_room/floor_lamp",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();

        let q = StateQuery {
            room: Some("kitchen".into()),
            ..Default::default()
        };
        let entries = reader.query(&q).unwrap();
        assert_eq!(entries.len(), 2);
        for e in &entries {
            assert_eq!(e.device_id.room().as_str(), "kitchen");
        }
    }

    #[test]
    fn device_id_wins_over_room() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let now = Utc::now();
        writer
            .append(&make_entry(
                now,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                now,
                "living_room/floor_lamp",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();

        let q = StateQuery {
            device_id: Some(make_id("kitchen/ceiling_light")),
            room: Some("living_room".into()),
            ..Default::default()
        };
        let entries = reader.query(&q).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, make_id("kitchen/ceiling_light"));
    }

    #[test]
    fn query_by_since_until() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let d1 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let d2 = Utc.with_ymd_and_hms(2026, 1, 2, 10, 0, 0).unwrap();
        let d3 = Utc.with_ymd_and_hms(2026, 1, 3, 10, 0, 0).unwrap();
        writer
            .append(&make_entry(
                d1,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                d2,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                d3,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();

        let q = StateQuery {
            since: Some(d2),
            until: Some(d3),
            ..Default::default()
        };
        let entries = reader.query(&q).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].ts, d3);
        assert_eq!(entries[1].ts, d2);
    }

    #[test]
    fn limit_default_and_clamp() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let now = Utc::now();
        for i in 0..250 {
            writer
                .append(&make_entry(
                    now + chrono::Duration::seconds(i),
                    "kitchen/ceiling_light",
                    DeviceState {
                        brightness: Some(i as u8),
                        ..Default::default()
                    },
                ))
                .unwrap();
        }

        let entries = reader.query(&StateQuery::default()).unwrap();
        assert_eq!(entries.len(), 200); // default

        let q10 = StateQuery {
            limit: Some(10),
            ..Default::default()
        };
        assert_eq!(reader.query(&q10).unwrap().len(), 10);

        let q3000 = StateQuery {
            limit: Some(3000),
            ..Default::default()
        };
        assert_eq!(reader.query(&q3000).unwrap().len(), 250); // clamped to 2000 but only 250 exist
    }

    #[test]
    fn snapshot_at_simple() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let id = make_id("kitchen/ceiling_light");
        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 1, 1, 11, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();

        writer
            .append(&make_entry(
                t1,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                t2,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                t3,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();

        let at_t3 = reader.snapshot_at(t3, std::slice::from_ref(&id)).unwrap();
        assert_eq!(at_t3.len(), 1);
        assert_eq!(at_t3[0].ts, t3);
        assert_eq!(at_t3[0].state.on, Some(false));

        let at_t2_5 = reader
            .snapshot_at(
                t2 + chrono::Duration::minutes(30),
                std::slice::from_ref(&id),
            )
            .unwrap();
        assert_eq!(at_t2_5.len(), 1);
        assert_eq!(at_t2_5[0].ts, t2);
        assert_eq!(at_t2_5[0].state.on, Some(true));

        let at_t0 = reader
            .snapshot_at(t1 - chrono::Duration::hours(1), std::slice::from_ref(&id))
            .unwrap();
        assert!(at_t0.is_empty());
    }

    #[test]
    fn snapshot_at_mixed() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        writer
            .append(&make_entry(
                t1,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();

        let a = make_id("kitchen/ceiling_light");
        let b = make_id("living_room/floor_lamp");
        let at = t1 + chrono::Duration::hours(1);

        let entries = reader.snapshot_at(at, &[a, b]).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].device_id, make_id("kitchen/ceiling_light"));
    }

    #[test]
    fn snapshot_at_picks_latest_per_device() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 1, 1, 11, 0, 0).unwrap();

        writer
            .append(&make_entry(
                t1,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                t2,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();

        let at = t2 + chrono::Duration::minutes(30);
        let entries = reader
            .snapshot_at(at, &[make_id("kitchen/ceiling_light")])
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ts, t2);
        assert_eq!(entries[0].state.on, Some(true));
    }

    #[test]
    fn retention_prune() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();

        let old = Utc::now() - chrono::Duration::days(20);
        let recent = Utc::now() - chrono::Duration::days(1);
        writer
            .append(&make_entry(
                old,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                recent,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();

        writer.prune(14).unwrap();

        let reader = StateReader::new(tmp.path());
        let entries = reader.query(&StateQuery::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].state.on, Some(false));
    }

    #[test]
    fn corrupted_line_tolerance() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let now = Utc::now();
        let t1 = now;
        let t2 = now + chrono::Duration::seconds(1);
        writer
            .append(&make_entry(
                t1,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();

        let path = tmp
            .path()
            .join("state")
            .join(format!("{}.jsonl", now.date_naive().format("%Y-%m-%d")));
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"this is not json\n").unwrap();
        }

        writer
            .append(&make_entry(
                t2,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();

        let entries = reader.query(&StateQuery::default()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].state.on, Some(false));
        assert_eq!(entries[1].state.on, Some(true));
    }

    #[test]
    fn disabled_writer_is_no_op() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::disabled();
        let entry = make_entry(Utc::now(), "kitchen/ceiling_light", DeviceState::default());
        writer.append(&entry).unwrap();
        writer.prune(1).unwrap();

        assert!(!tmp.path().join("state").exists());
    }

    #[test]
    fn disabled_reader_returns_empty() {
        let reader = StateReader::disabled();
        let entries = reader.query(&StateQuery::default()).unwrap();
        assert!(entries.is_empty());

        let entries = reader
            .snapshot_at(Utc::now(), &[make_id("kitchen/ceiling_light")])
            .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn since_greater_than_until_errors() {
        let tmp = TempDir::new().unwrap();
        let reader = StateReader::new(tmp.path());

        let q = StateQuery {
            since: Some(Utc::now()),
            until: Some(Utc::now() - chrono::Duration::hours(1)),
            ..Default::default()
        };
        let err = reader.query(&q).unwrap_err();
        assert!(matches!(err, Error::InvalidDateRange { .. }));
    }

    #[test]
    fn concurrent_appends_are_atomic() {
        let tmp = TempDir::new().unwrap();
        let writer = Arc::new(StateWriter::new(tmp.path()).unwrap());
        let reader = StateReader::new(tmp.path());

        let now = Utc::now();
        let mut handles = Vec::new();
        for t in 0..10 {
            let w = writer.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..20 {
                    w.append(&make_entry(
                        now + chrono::Duration::seconds(t * 20 + i),
                        "kitchen/ceiling_light",
                        DeviceState {
                            brightness: Some((t * 20 + i) as u8),
                            ..Default::default()
                        },
                    ))
                    .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let entries = reader
            .query(&StateQuery {
                limit: Some(2000),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(entries.len(), 200);
        let brightnesses: std::collections::HashSet<_> =
            entries.iter().map(|e| e.state.brightness).collect();
        assert_eq!(brightnesses.len(), 200);
    }

    #[test]
    fn missing_dir_at_construction() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("nested");
        assert!(!root.exists());
        let writer = StateWriter::new(&root).unwrap();
        assert!(root.join("state").exists());
        writer
            .append(&make_entry(
                Utc::now(),
                "kitchen/ceiling_light",
                DeviceState::default(),
            ))
            .unwrap();
    }

    #[test]
    fn reader_on_empty_or_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let reader = StateReader::new(tmp.path());
        let entries = reader.query(&StateQuery::default()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn snapshot_at_empty_device_ids_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let t = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        writer
            .append(&make_entry(
                t,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();

        let entries = reader.snapshot_at(t, &[]).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn reader_skips_directories_in_state_dir() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let now = Utc::now();
        writer
            .append(&make_entry(
                now,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();

        // A stray directory should not break queries.
        let state_dir = tmp.path().join("state");
        let stray = state_dir.join("2026-01-01");
        std::fs::create_dir(&stray).unwrap();

        let entries = reader.query(&StateQuery::default()).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn snapshot_at_across_days() {
        let tmp = TempDir::new().unwrap();
        let writer = StateWriter::new(tmp.path()).unwrap();
        let reader = StateReader::new(tmp.path());

        let day1 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 1, 2, 10, 0, 0).unwrap();
        let day3 = Utc.with_ymd_and_hms(2026, 1, 3, 12, 0, 0).unwrap();

        writer
            .append(&make_entry(
                day1,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                day2,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
            ))
            .unwrap();
        writer
            .append(&make_entry(
                day3,
                "kitchen/ceiling_light",
                DeviceState {
                    on: Some(false),
                    ..Default::default()
                },
            ))
            .unwrap();

        let id = make_id("kitchen/ceiling_light");

        let at_day3 = reader.snapshot_at(day3, std::slice::from_ref(&id)).unwrap();
        assert_eq!(at_day3.len(), 1);
        assert_eq!(at_day3[0].ts, day3);
        assert_eq!(at_day3[0].state.on, Some(false));

        let at_day2_5 = reader
            .snapshot_at(
                day2 + chrono::Duration::hours(12),
                std::slice::from_ref(&id),
            )
            .unwrap();
        assert_eq!(at_day2_5.len(), 1);
        assert_eq!(at_day2_5[0].ts, day2);
        assert_eq!(at_day2_5[0].state.on, Some(true));

        let at_day1_5 = reader
            .snapshot_at(
                day1 + chrono::Duration::hours(12),
                std::slice::from_ref(&id),
            )
            .unwrap();
        assert_eq!(at_day1_5.len(), 1);
        assert_eq!(at_day1_5[0].ts, day1);
        assert_eq!(at_day1_5[0].state.on, Some(false));
    }
}
