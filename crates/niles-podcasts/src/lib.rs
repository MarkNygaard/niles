//! niles-podcasts — researching an episode is the assistant's job;
//! this crate is everything after: saying the script in two Kokoro
//! voices, joining the MP3 frames, keeping the file in the house's own
//! R2 bucket, and playing it back.
//!
//! Frames are joined, never re-encoded: Kokoro already encodes, and
//! joining is byte arithmetic, so no MP3 encoder (and no C dependency)
//! ships here. The bucket is reached path-style
//! (`{endpoint}/{bucket}/{key}`), the shape R2 uses. Playback URLs are
//! presigned GETs because the players that fetch them — a browser's
//! `<audio>` element, a Sonos speaker — can sign nothing themselves.

mod bucket;
mod check;
mod episode;
mod error;
mod kokoro;
pub mod mp3;
mod sigv4;
mod studio;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::LazyLock;
use std::time::Duration;

pub use bucket::{Bucket, BucketSettings};
pub use check::{Check, Part, check};
pub use episode::{LEAD_IN, PAUSE, PLAY_FOR, episode_key, record};
pub use error::{Error, Result};
pub use kokoro::Kokoro;
pub use mp3::Joined;
pub use studio::{Studio, missing, voice_hint};

/// Shared HTTP client: keep-alive across the dozen calls one episode
/// makes. Timeouts are per request, not here; the connect timeout is
/// the one that catches "nothing answered at all" fast enough to retry.
static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("Niles podcasts")
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("a reqwest client builds")
});

/// A voice: turns one line of the script into MP3 audio.
#[async_trait]
pub trait Speech: Send + Sync {
    async fn say(&self, voice: &str, text: &str) -> Result<Vec<u8>>;
}

/// Where a finished episode is kept, and how it is reached.
#[async_trait]
pub trait Storage: Send + Sync {
    async fn put(&self, key: &str, body: Vec<u8>, content_type: &str) -> Result<()>;
    async fn delete(&self, key: &str) -> Result<()>;
    /// A time-limited GET address for `key`. The URL names the access
    /// key id, never the secret; it is never logged.
    fn presign_get(&self, key: &str, valid_for: Duration, now: DateTime<Utc>) -> String;
}
