//! niles-capabilities — capability reference loader for tiered LLM context assembly.
//!
//! Scans a capability directory for subdirectories containing `SKILL.md` files
//! (agentskills.io / Hermes Agent convention), parses YAML frontmatter +
//! markdown body, and exposes indexed capabilities to callers.

mod error;
mod loader;
mod skill;

pub use error::{Error, Result};
pub use loader::CapabilityLoader;
pub use skill::{Capability, CapabilityMetadata};
