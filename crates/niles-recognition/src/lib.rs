//! niles-recognition — speaker identification via voice embeddings.
//!
//! v1 ships the inference layer only: an ECAPA-TDNN ONNX-Runtime
//! embedder + cosine similarity helper. Enrollment + matching land in
//! a follow-up PR.

pub mod embedder;
pub mod error;
pub mod preprocess;
pub mod similarity;

pub use embedder::{EcapaTdnnEmbedder, EmbedderConfig};
pub use error::{Error, Result};
pub use similarity::{cosine_similarity, l2_normalize};
