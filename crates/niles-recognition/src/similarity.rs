//! Cosine similarity + L2 normalization for speaker embeddings.

/// Cosine similarity between two embeddings.
///
/// Both inputs MUST already be L2-normalized; [`crate::EcapaTdnnEmbedder::extract`]
/// guarantees this. Range: -1.0..=1.0. Higher = more similar.
///
/// Panics if lengths differ — callers must pass two embeddings from the
/// same model.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "embedding length mismatch");
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// In-place L2 normalize. No-op for zero vectors (avoids NaN).
pub fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_normalized_vectors() {
        let a = [1.0_f32, 0.0, 0.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors() {
        let a = [1.0_f32, 0.0];
        let b = [0.0_f32, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors() {
        let a = [1.0_f32, 0.0];
        let b = [-1.0_f32, 0.0];
        assert!((cosine_similarity(&a, &b) - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_3_4() {
        let mut v = [3.0_f32, 4.0];
        l2_normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_zero_is_noop() {
        let mut v = [0.0_f32, 0.0];
        l2_normalize(&mut v);
        assert_eq!(v, [0.0, 0.0]);
    }

    #[test]
    fn cosine_similarity_of_zero_vectors() {
        let a = [0.0_f32, 0.0, 0.0];
        let b = [0.0_f32, 0.0, 0.0];
        // l2_normalize is a no-op for zero vectors, so dot product is 0
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }
}
