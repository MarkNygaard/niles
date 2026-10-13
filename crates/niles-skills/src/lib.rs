//! niles-skills — agent-mintable skill store (write side).
//!
//! Provides `SkillStore` for creating, loading, patching, deleting,
//! and listing skills, with security scanning and usage telemetry.
//! Every change is handed to a [`SkillPersistence`]; Niles keeps the
//! set in Postgres.

pub mod curator;
pub mod error;
pub mod scan;
pub mod sidecar;
pub mod store;

pub use error::{Error, Result};
pub use sidecar::{Provenance, Sidecar, SkillStatus};
pub use store::{Skill, SkillPersistence, SkillStatusFilter, SkillStore, SkillSummary};
