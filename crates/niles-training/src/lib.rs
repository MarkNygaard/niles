//! niles-training — each person's workouts, from their own Strava.
//!
//! Why Strava is in ARCHITECTURE.md's Operations notes. Each person
//! connects their own Strava on their profile, with an API app they
//! made themselves ([`StravaApp`]); [`Strava`] brings their activities in
//! and keeps them coming, in canonical units — metres, seconds, beats
//! per minute, kilocalories.
//!
//! Everything here belongs to one person, named by the same key as
//! their chat history ([`person_key`]), so what they ask by voice and
//! what they ask in the app reach the same data, and nobody else's.

mod error;
mod model;
mod store;
pub mod strava;
mod summary;
mod sync;

pub use error::{Error, Result};
pub use model::{Activity, Athlete, Kind, StravaApp, StravaLink, StravaTokens, person_key};
pub use store::{MemoryTraining, Overview, Saved, TrainingStore};
pub use summary::{Summary, summarise};
pub use sync::{Arrived, BACKFILL, OVERLAP, POLL, Strava, Synced};
