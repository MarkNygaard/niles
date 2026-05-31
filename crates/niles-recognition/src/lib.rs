//! niles-recognition — speaker identification via voice embeddings.
//!
//! v1 ships the inference layer (ECAPA-TDNN ONNX-Runtime embedder +
//! cosine similarity helper); v2 adds enrollment persistence and
//! speaker matching.

pub mod embedder;
pub mod enrollment;
pub mod error;
pub mod matcher;
pub mod preprocess;
pub mod similarity;

pub use embedder::{EcapaTdnnEmbedder, EmbedderConfig};
pub use enrollment::{EnrolledSpeaker, EnrollmentEntry, EnrollmentStore};
pub use error::{Error, Result};
pub use matcher::{MatchOutcome, MatchStrategy, Matcher};
pub use similarity::{cosine_similarity, l2_normalize};
