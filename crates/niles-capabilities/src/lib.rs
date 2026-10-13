//! niles-capabilities — capability reference loader for tiered LLM context assembly.
//!
//! Parses `SKILL.md` files (agentskills.io / Hermes Agent convention) —
//! YAML frontmatter + markdown body — and exposes indexed capabilities
//! to callers. The references in `docs/capabilities` are compiled into
//! the crate; see [`CapabilityLoader::builtin`].

mod error;
mod loader;
mod skill;

pub use error::{Error, Result};
pub use loader::CapabilityLoader;
pub use skill::{Capability, CapabilityMetadata};
