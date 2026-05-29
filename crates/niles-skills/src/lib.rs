//! niles-skills — agent-mintable skill store (write side).
//!
//! Provides `SkillStore` for creating, loading, patching, deleting,
//! and listing skills with atomic writes, OS advisory locking,
//! security scanning, and `.usage.json` sidecar telemetry.

pub mod curator;
pub mod error;
pub mod scan;
pub mod sidecar;
pub mod store;
pub(crate) mod util;

pub use error::{Error, Result};
pub use sidecar::{Provenance, Sidecar, SkillStatus};
pub use store::{Skill, SkillStatusFilter, SkillStore, SkillSummary};
