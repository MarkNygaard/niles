//! niles-artifacts — what Niles makes for one person and keeps for
//! them: a travel plan, a packing list, later a training plan.
//!
//! See ARCHITECTURE.md § Artifacts (per person). An artifact is one
//! person's, under the same key as their chat ([`person_key`] in
//! `niles-training`), and every version of its content is kept, up to
//! [`KEPT_REVISIONS`]. What the content looks like is its kind's
//! business ([`kinds`]): a document is markdown.

mod document;
mod error;
pub mod kinds;
mod model;
mod store;

pub use error::{Error, Result};
pub use kinds::{Kind, Preview, PreviewLine, Style};
pub use model::{
    Artifact, ArtifactRef, Author, Changes, Filter, NewArtifact, NewRevision, Revision,
    RevisionMeta, Status, StatusChange,
};
pub use store::{ArtifactStore, KEPT_REVISIONS, MemoryArtifacts, restore};
