//! SHA-256 rolling page hash for L3 storage lookup.

use sha2::{Digest, Sha256};

/// Compute a SHA-256 hash of a token page, chaining from a prior hash.
///
/// Returns the hex-encoded hash string.
pub fn hash_page(tokens: &[i32], prior_hash: &str) -> String {
    let mut hasher = Sha256::new();
    if !prior_hash.is_empty() {
        if let Ok(prior_bytes) = hex::decode(prior_hash) {
            hasher.update(&prior_bytes);
        }
    }
    for &token in tokens {
        hasher.update(&token.to_le_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Compute rolling page hashes for a sequence of token pages.
pub fn compute_paged_hashes(token_pages: &[&[i32]]) -> Vec<String> {
    let mut hashes = Vec::with_capacity(token_pages.len());
    let mut prior = String::new();
    for page in token_pages {
        let h = hash_page(page, &prior);
        hashes.push(h.clone());
        prior = h;
    }
    hashes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_page_empty_prior() {
        let h = hash_page(&[1, 2, 3, 4], "");
        assert_eq!(h.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn test_hash_page_deterministic() {
        let a = hash_page(&[1, 2], "");
        let b = hash_page(&[1, 2], "");
        assert_eq!(a, b);
    }

    #[test]
    fn test_compute_paged_hashes() {
        let pages: &[&[i32]] = &[&[1, 2], &[3, 4]];
        let hashes = compute_paged_hashes(pages);
        assert_eq!(hashes.len(), 2);
        // Chained: each hash depends on the prior
        assert_ne!(hashes[0], hashes[1]);
    }
}
