//! niles-memory — Hermes-style persistent markdown memory.
//!
//! Two files live in a configured directory:
//! - `USER.md`  — facts about the household humans
//! - `MEMORY.md` — the agent's own learnings
//!
//! Entries are §-delimited.  All writes are atomic (tempfile-rename
//! + OS advisory file lock) and gated by a security scan.

pub mod error;
pub mod scan;
pub mod store;

pub use error::{Error, Result};
pub use store::{Entry, MemoryConfig, MemoryStore, Target};
