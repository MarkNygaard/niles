//! niles-intent — Tier 0 regex intent router.
//!
//! Maps voice transcripts to structured `Intent`s without touching an LLM.
//! Anything that doesn't match falls through to the Tier 1 LLM router
//! (caller's responsibility — `IntentRouter::parse` returns `None`).
//!
//! The router is purely a pattern matcher: it produces `Intent`s with
//! raw text references (room names as the user said them, like
//! `"living room"`). Resolution against the device registry happens
//! at a higher layer.

pub mod intent;
pub mod router;
pub mod topic;

pub use intent::Intent;
pub use router::IntentRouter;
pub use topic::{CapabilityIndex, CapabilityIndexEntry, detect_topics};
