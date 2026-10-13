//! niles-memory — Hermes-style persistent markdown memory.
//!
//! Two documents:
//! - `user`  — facts about the household humans
//! - `agent` — Niles's own learnings
//!
//! Entries are §-delimited, and every write is gated by a security
//! scan. Niles keeps both in Postgres.

pub mod error;
pub mod scan;
pub mod store;

pub use error::{Error, Result};
pub use store::{Entry, Forgotten, MemoryConfig, MemoryPersistence, MemoryStore, Target, similar};
