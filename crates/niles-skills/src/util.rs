//! Internal helpers.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::error::Result;

/// Atomically write `content` to `path` using a temp file + rename.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let suffix = format!(
        "tmp.{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let tmp = path.with_extension(&suffix);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)?;
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}
