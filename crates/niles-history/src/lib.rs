//! Command history persistence — JSONL append per day, queryable by
//! the Tier-1 LLM to resolve anaphora and retrospective questions.

pub mod command;
pub mod error;
pub mod state;

pub use command::{CommandEntry, CommandQuery, CommandReader, CommandWriter};
pub use error::{Error, Result};
pub use state::{StateEntry, StateQuery, StateReader, StateWriter};
