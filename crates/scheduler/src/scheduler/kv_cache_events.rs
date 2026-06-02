//! KV cache event types and FNV-1a block hashing.

/// KV cache event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvCacheEventKind {
    BlockStored,
    BlockRemoved,
}

/// A KV cache event emitted by the scheduler.
#[derive(Debug, Clone)]
pub struct KvCacheEvent {
    pub kind: KvCacheEventKind,
    pub block_hash: u64,
    pub parent_block_hash: Option<u64>,
    pub token_ids: Vec<i32>,
    pub page_ids: Vec<i32>,
}

/// FNV-1a 64-bit hash of token IDs with optional parent hash chaining.
pub fn hash_kv_block(token_ids: &[i32], parent_hash: Option<u64>) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = parent_hash.unwrap_or(FNV_OFFSET_BASIS);
    for &token in token_ids {
        let bytes = token.to_le_bytes();
        for &byte in &bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fnv_hash_deterministic() {
        let a = hash_kv_block(&[1, 2, 3], None);
        let b = hash_kv_block(&[1, 2, 3], None);
        assert_eq!(a, b);
    }

    #[test]
    fn test_fnv_hash_different_inputs() {
        let a = hash_kv_block(&[1, 2, 3], None);
        let b = hash_kv_block(&[1, 2, 4], None);
        assert_ne!(a, b);
    }

    #[test]
    fn test_fnv_hash_with_parent() {
        // FNV-1a chaining: hash([3,4], parent_hash=hash([1,2])) == hash([1,2,3,4])
        let parent = hash_kv_block(&[1, 2], None);
        let child = hash_kv_block(&[3, 4], Some(parent));
        let combined = hash_kv_block(&[1, 2, 3, 4], None);
        assert_eq!(child, combined);
    }
}
