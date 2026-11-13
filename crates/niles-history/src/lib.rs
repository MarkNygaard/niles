//! The house's recent past, which the Tier-1 model reads to resolve
//! anaphora ("turn it off again") and to answer, or put back, how
//! things were ("set the lights like yesterday at eight").
//!
//! Two histories: each device's whole state after each change
//! ([`state`]), and each spoken turn ([`command`]). Each is a trait
//! with an in-memory version here; the one a house runs on is in
//! Postgres (`niles-db`), so it outlives a restart. Without a database
//! there is no history, and the tools that read it are not offered.

pub mod command;
pub mod error;
pub mod state;

pub use command::{CommandEntry, CommandHistory, CommandQuery, MemoryCommandHistory};
pub use error::{Error, Result};
pub use state::{MemoryStateHistory, StateEntry, StateHistory, StateQuery};

/// How long either history is kept, in days.
pub const KEPT_DAYS: i64 = 14;

/// Refuse a range that ends before it starts.
fn check_range(
    since: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<()> {
    match (since, until) {
        (Some(since), Some(until)) if since > until => Err(Error::InvalidDateRange {
            reason: "since > until".into(),
        }),
        _ => Ok(()),
    }
}
