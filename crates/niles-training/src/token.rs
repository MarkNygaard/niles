//! The ingest key, as it is kept: a SHA-256 of it, never the key.
//!
//! A plain hash rather than a slow one, because the key is 32 random
//! bytes rather than something a person chose: there is nothing for a
//! dictionary to guess, so a stolen table of hashes gives nothing back.

use sha2::{Digest, Sha256};

/// What is kept for `key`, and what a presented key is looked up by.
pub fn hash_key(key: &str) -> String {
    Sha256::digest(key.trim().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether two hashes are the same, without returning early at the
/// first difference.
pub fn same_hash(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_not_the_key_and_is_stable() {
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let hash = hash_key(key);
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, key);
        assert_eq!(hash, hash_key(key));
        assert_ne!(hash, hash_key("another key"));
    }

    #[test]
    fn hashes_are_compared_whole() {
        let a = hash_key("a");
        assert!(same_hash(&a, &hash_key("a")));
        assert!(!same_hash(&a, &hash_key("b")));
        assert!(!same_hash(&a, &a[..63]));
        assert!(!same_hash("", "x"));
    }
}
