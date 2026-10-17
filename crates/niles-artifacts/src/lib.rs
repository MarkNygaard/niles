//! niles-artifacts — what Niles makes for one person and keeps for
//! them: a travel plan, a packing list, a training plan.
//!
//! An artifact is one person's, under the same key as their chat
//! ([`person_key`] in `niles-training`), and every version of its content is kept, up to
//! [`KEPT_REVISIONS`]. What the content looks like is its kind's
//! business ([`kinds`]): a document is markdown, a training plan
//! weeks of dated sessions, a trip its days and places. An artifact may
//! also have a [`page`] that draws that content: kept once, and named
//! by each version drawn with it, and [`images`]: Niles's own copies of
//! pictures, which its content and page name by id. Its owner may
//! [`share`] it with others in the household, to read or to edit.

mod document;
mod error;
pub mod images;
pub mod kinds;
mod model;
pub mod page;
pub mod plan_day;
pub mod share;
mod store;
pub mod tracking;
mod training_plan;
mod trip;

pub use error::{Error, Result};
pub use images::{Image, ImageData, NewImage};
pub use kinds::{Kind, Preview, PreviewLine, Style};
pub use model::{
    Artifact, ArtifactRef, Author, Changes, Filter, NewArtifact, NewRevision, PageChange, Revision,
    RevisionMeta, Status, StatusChange, summary, title,
};
pub use share::{Access, Level, Opened, Operation, Share, open};
pub use store::{ArtifactStore, KEPT_REVISIONS, MemoryArtifacts, restore};
