//! File-based notification log: JSONL partitioned by UTC date.

use crate::error::Result;
use crate::model::Notification;
use chrono::{NaiveDate, Utc};
use parking_lot::Mutex;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// Append-only JSONL notification log, partitioned by UTC date.
pub struct NotificationLog {
    root: PathBuf,
    enabled: bool,
    lock: Mutex<()>,
}

impl NotificationLog {
    /// Create a log that persists under `root/notifications/`.
    /// Creates the subdirectory if it does not exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let dir = root.join("notifications");
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            root,
            enabled: true,
            lock: Mutex::new(()),
        })
    }

    /// No-op log that never touches the filesystem.
    pub fn disabled() -> Self {
        Self {
            root: PathBuf::new(),
            enabled: false,
            lock: Mutex::new(()),
        }
    }

    /// Append `notification` to today's JSONL file.
    pub fn append(&self, n: &Notification) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let _guard = self.lock.lock();
        let date = n.created_at.date_naive();
        let path = self
            .root
            .join("notifications")
            .join(format!("{}.jsonl", date.format("%Y-%m-%d")));
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut buf = Vec::new();
        serde_json::to_writer(&mut buf, n)?;
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
        let notifications_dir = self.root.join("notifications");
        let entries = match std::fs::read_dir(&notifications_dir) {
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
                    tracing::debug!(
                        "skipping non-date file in notification log dir: {}",
                        path.display()
                    );
                    continue;
                }
            };
            if date < oldest_kept {
                match std::fs::remove_file(&path) {
                    Ok(()) => tracing::info!("pruned old notification log: {}", path.display()),
                    Err(e) => tracing::warn!("failed to prune {}: {e}", path.display()),
                }
            }
        }
        Ok(())
    }

    /// Load the most recent notifications, newest first.
    pub fn load_recent(&self, limit: usize) -> Result<Vec<Notification>> {
        if !self.enabled || limit == 0 {
            return Ok(Vec::new());
        }
        let notifications_dir = self.root.join("notifications");
        let dir_entries = match std::fs::read_dir(&notifications_dir) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut files: Vec<_> = dir_entries
            .filter_map(|e| e.ok())
            .filter_map(|entry| {
                let path = entry.path();
                let stem = path.file_stem()?.to_str()?;
                let date = NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()?;
                Some((date, path))
            })
            .collect();
        files.sort_by_key(|(date, _)| std::cmp::Reverse(*date));

        let mut results = Vec::new();
        for (_, path) in files {
            if results.len() >= limit {
                break;
            }
            let file = std::fs::File::open(&path)?;
            let reader = BufReader::new(file);
            let mut day = Vec::new();
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Notification>(&line) {
                    Ok(n) => day.push(n),
                    Err(e) => {
                        tracing::warn!(
                            "skip malformed notification line in {}: {e}",
                            path.display()
                        );
                    }
                }
            }
            for n in day.into_iter().rev() {
                results.push(n);
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeliveryOutcome, Priority};
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn make_notification(ts: chrono::DateTime<Utc>, text: &str) -> Notification {
        Notification {
            id: "test-id".into(),
            text: text.into(),
            priority: Priority::Routine,
            room: None,
            outcome: DeliveryOutcome::Delivered,
            created_at: ts,
        }
    }

    #[test]
    fn round_trip() {
        let tmp = TempDir::new().unwrap();
        let log = NotificationLog::new(tmp.path()).unwrap();

        let now = Utc::now();
        for i in 1..=3 {
            let n = make_notification(now + chrono::Duration::seconds(i), &format!("msg {i}"));
            log.append(&n).unwrap();
        }

        let recent = log.load_recent(10).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].text, "msg 3");
        assert_eq!(recent[1].text, "msg 2");
        assert_eq!(recent[2].text, "msg 1");
    }

    #[test]
    fn daily_rotation() {
        let tmp = TempDir::new().unwrap();
        let log = NotificationLog::new(tmp.path()).unwrap();

        let day1 = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap();
        log.append(&make_notification(day1, "day one")).unwrap();
        log.append(&make_notification(day2, "day two")).unwrap();

        let notifications_dir = tmp.path().join("notifications");
        assert!(notifications_dir.join("2026-01-01.jsonl").exists());
        assert!(notifications_dir.join("2026-01-02.jsonl").exists());

        let recent = log.load_recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "day two");
        assert_eq!(recent[1].text, "day one");
    }

    #[test]
    fn prune_removes_old() {
        let tmp = TempDir::new().unwrap();
        let log = NotificationLog::new(tmp.path()).unwrap();

        let old = Utc::now() - chrono::Duration::days(20);
        let recent = Utc::now() - chrono::Duration::days(1);
        log.append(&make_notification(old, "old")).unwrap();
        log.append(&make_notification(recent, "recent")).unwrap();

        log.prune(14).unwrap();

        let loaded = log.load_recent(10).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].text, "recent");
    }

    #[test]
    fn disabled_is_no_op() {
        let log = NotificationLog::disabled();
        let n = make_notification(Utc::now(), "x");
        log.append(&n).unwrap();
        log.prune(7).unwrap();
        let recent = log.load_recent(10).unwrap();
        assert!(recent.is_empty());
    }

    #[test]
    fn corrupted_line_tolerated() {
        let tmp = TempDir::new().unwrap();
        let log = NotificationLog::new(tmp.path()).unwrap();

        let now = Utc::now();
        log.append(&make_notification(now, "first")).unwrap();
        let date = now.date_naive();
        let path = tmp
            .path()
            .join("notifications")
            .join(format!("{}.jsonl", date.format("%Y-%m-%d")));
        {
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(b"this is not json\n").unwrap();
        }
        let later = now + chrono::Duration::seconds(1);
        log.append(&make_notification(later, "second")).unwrap();

        let recent = log.load_recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "second");
        assert_eq!(recent[1].text, "first");
    }
}
