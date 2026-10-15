//! niles-training — each person's runs, sleep, heart rate and weight.
//!
//! See ARCHITECTURE.md § Training (per person). What arrives is what a
//! phone already knows: the Garmin watch and scale write to Apple
//! Health, and the Health Auto Export app posts Apple Health to Niles.
//! [`hae`] reads that post, in canonical units — metres, seconds,
//! kilograms, beats per minute, milliseconds — whatever units it came
//! in.
//!
//! Everything here belongs to one person, named by the same key as
//! their chat history ([`person_key`]), so what they ask by voice and
//! what they ask in the app reach the same data, and nobody else's.

mod error;
pub mod hae;
mod model;
mod store;
mod summary;
mod token;

pub use error::{Error, Result};
pub use model::{Activity, Kind, Wellness, person_key};
pub use store::{MemoryTraining, Overview, Saved, TrainingStore};
pub use summary::{Summary, Trend, WeightChange, summarise};
pub use token::{hash_key, same_hash};
