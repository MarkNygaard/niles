//! Where enrolled voices are kept.
//!
//! [`Matcher`](crate::Matcher) holds the enrolled speakers in memory
//! and compares against them on the CPU, so the hot path never touches
//! storage. This trait covers the rest: loading them at startup,
//! enrolling a new one, and recording that someone was heard.
//!
//! # Why async, when the file implementation isn't
//!
//! Because the other implementation is a database. Reading a few
//! kilobytes of JSON from a local directory needs no async at all, but
//! a deployment that cannot keep a local directory — a pod with a
//! read-only root filesystem, which is the normal shape — has to put
//! them somewhere reachable over a socket.

use crate::EnrolledSpeaker;
use crate::error::Result;
use async_trait::async_trait;

/// Durable storage for enrolled voices.
/// The enrolled voices, for showing and for forgetting.
///
/// Narrower than [`EnrollmentBackend`] on purpose: the app needs to see
/// who is enrolled and to delete one, and has no business adding clips
/// or reading embeddings. It is also not the backend itself, because
/// forgetting somebody has to rebuild the live matcher as well as the
/// store — deleting only from the store would leave Niles recognising
/// a voice it had been told to forget until the next restart.
#[async_trait]
pub trait VoiceRoster: Send + Sync {
    /// Everybody enrolled, for display.
    async fn voices(&self) -> Result<Vec<EnrolledSpeaker>>;

    /// Forget one entirely, store and matcher both.
    async fn forget(&self, speaker: &str) -> Result<()>;

    /// Give one a different display name.
    ///
    /// The slug stays: it is what `auth.allowed[].speaker` points at
    /// and what the clips are filed under, and renaming *that* would
    /// break a pairing to fix a spelling. Only the name a person reads
    /// changes — which is the half Whisper got wrong.
    async fn rename(&self, speaker: &str, display_name: &str) -> Result<()>;
}

#[async_trait]
pub trait EnrollmentBackend: Send + Sync {
    /// Every enrolled speaker. Read once at startup to build the
    /// matcher, so a new enrollment needs a restart to take effect.
    async fn load_all(&self) -> Result<Vec<EnrolledSpeaker>>;

    /// Add a clip's embedding to a speaker, creating them if new.
    async fn enroll(&self, speaker: &str, embedding: &[f32]) -> Result<()>;

    /// One speaker's record.
    async fn load(&self, speaker: &str) -> Result<EnrolledSpeaker>;

    /// The enrolled speaker slugs.
    async fn list(&self) -> Result<Vec<String>>;

    /// Forget a speaker entirely.
    async fn delete(&self, speaker: &str) -> Result<()>;

    /// Set the display name, leaving the slug and the clips alone.
    async fn set_display_name(&self, speaker: &str, display_name: &str) -> Result<()>;

    /// Record that this speaker was just heard.
    ///
    /// Only ever used for display. A lost write costs a timestamp, not
    /// a recognition, which is why the caller is free to fire this at a
    /// background task rather than wait for it.
    async fn bump_last_seen(&self, speaker: &str) -> Result<()>;

    /// Short description for logs — "the directory /var/lib/niles",
    /// "the niles database".
    fn describe(&self) -> String;
}
