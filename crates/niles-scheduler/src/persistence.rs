//! Shared persistence helpers for scheduler stores.

use serde::{Serialize, de::DeserializeOwned};
use std::io::{ErrorKind, Result};
use std::path::Path;

/// Atomically write `value` as JSON to `path`.
///
/// Uses `tempfile::NamedTempFile` in the target directory, calls
/// `sync_all()`, then renames into place. A crash between
/// `sync_all` and `persist` leaves the prior file intact.
pub(crate) fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if dir != Path::new(".") {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(tmp.as_file_mut(), value).map_err(std::io::Error::other)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    // Ensure the directory entry is durable on POSIX.
    if let Ok(dir_file) = std::fs::File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

/// Read JSON from `path`, returning `T::default()` on missing or
/// corrupt files.
///
/// Missing files are expected on first start. Corrupt files are
/// logged and recovered-from by starting empty.
pub(crate) fn read_json_or_empty<T: DeserializeOwned + Default>(
    path: &Path,
    label: &'static str,
) -> Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<T>(&bytes) {
            Ok(v) => Ok(v),
            Err(e) => {
                tracing::warn!(
                    "persistence: {label} at {} corrupted ({e}); starting empty",
                    path.display()
                );
                Ok(T::default())
            }
        },
        Err(e) if e.kind() == ErrorKind::NotFound => {
            tracing::info!(
                "persistence: {label} at {} not found; starting empty",
                path.display()
            );
            Ok(T::default())
        }
        Err(e) => Err(e),
    }
}
