//! Shared types, type aliases, and enums used across the resource module.

use std::collections::HashMap;

/// Token ID type.
pub type TokenId = i32;
/// Vector of token IDs.
pub type TokenVec = Vec<TokenId>;
/// Cache operation ID.
pub type CacheOpId = u32;

/// Resource type: Device (GPU) or Host (CPU).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Device,
    Host,
}

/// Intent of a prefix-cache match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchIntent {
    /// Normal prefix reuse during prefill/decode.
    PrefixReuse,
    /// Recovery of state after retraction.
    StateRecovery,
}

/// Role of the scheduler instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Prefill-only node.
    P,
    /// Decode-only node.
    D,
    /// Fused prefill+decode node (default).
    Fused,
}

/// Disaggregation mode for prefill/decode separation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisaggregationMode {
    /// No disaggregation (fused mode).
    None,
    /// Prefill-only disaggregation.
    Prefill,
    /// Decode-only disaggregation.
    Decode,
}

// ---------------------------------------------------------------------------
// NodeKey — stable generational index for radix tree nodes.
// Defined here so both radix_tree and prefix_cache modules can use it
// without circular module dependencies.
// ---------------------------------------------------------------------------

slotmap::new_key_type! {
    /// Stable generational index for a radix tree node.
    pub struct NodeKey;
}

/// Result of a prefix-cache match.
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub device: MatchTier,
    pub host: MatchTier,
    /// Mamba branching seqlen (-1 = inactive).
    pub mamba_branching_seqlen: i32,
    /// Mamba COW source index (-1 = inactive).
    pub mamba_cow_src_index: i32,
    /// Mamba host source index (-1 = inactive).
    pub mamba_host_src_index: i32,
    /// Paged-cache adjunct hit info.
    pub paged_cache: PagedCacheMatch,
}

/// Per-tier match information.
#[derive(Debug, Clone)]
pub struct MatchTier {
    /// The deepest node on this tier that still has resource attached.
    pub last_node: Option<NodeKey>,
    pub page_size: i32,
}

impl MatchTier {
    pub fn depth_in_page(&self) -> i32 {
        // Depth is tracked per-node; callers should use the tree to query.
        0
    }
}

/// Paged-cache adjunct match information.
#[derive(Debug, Clone, Default)]
pub struct PagedCacheMatch {
    /// The deepest snapshot-bearing node found during walk.
    pub last_node: Option<NodeKey>,
    /// Number of tokens covered by the snapshot prefix.
    pub prefix_len_tokens: i32,
    /// Per-group page IDs from the snapshot match.
    pub per_group_page_ids: HashMap<String, Vec<i32>>,
    /// Per-group base logical page offset.
    pub per_group_base_logical_page: HashMap<String, i32>,
    /// How the snapshot was restored.
    pub restore_kind: PagedCacheRestoreKind,
    /// Phase 2 replay start tokens (phase 1 always 0).
    pub replay_start_tokens: i32,
}

/// How a paged-cache snapshot was restored during match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagedCacheRestoreKind {
    /// Full snapshot with all groups complete.
    SnapshotComplete,
}

impl Default for PagedCacheRestoreKind {
    fn default() -> Self {
        Self::SnapshotComplete
    }
}

/// Result of inserting tokens into the prefix cache.
#[derive(Debug, Clone)]
pub struct InsertResult {
    /// The last (deepest) tree node after insertion.
    pub last_node: NodeKey,
    /// Number of new pages inserted.
    pub inserted_num_pages: i32,
}

/// Result of splitting a tree node.
#[derive(Debug, Clone)]
pub struct SplitResult {
    pub parent: NodeKey,
    pub prefix: NodeKey,
    pub suffix: NodeKey,
}

/// Result of walking down the radix tree.
#[derive(Debug, Clone)]
pub struct WalkResult {
    /// The terminal node reached.
    pub terminal: NodeKey,
    /// Offset into the original token slice for remaining tokens.
    pub remaining_offset: usize,
    /// Accumulated match result.
    pub match_result: MatchResult,
}

/// Spec for a pending cache operation.
#[derive(Debug, Clone)]
pub struct CacheOpSpec {
    pub request_id: String,
    pub last_node: Option<NodeKey>,
    pub nodes: Vec<NodeKey>,
}

impl Default for CacheOpSpec {
    fn default() -> Self {
        Self {
            request_id: String::new(),
            last_node: None,
            nodes: Vec::new(),
        }
    }
}

impl Default for MatchResult {
    fn default() -> Self {
        Self {
            device: MatchTier {
                last_node: None,
                page_size: 0,
            },
            host: MatchTier {
                last_node: None,
                page_size: 0,
            },
            mamba_branching_seqlen: -1,
            mamba_cow_src_index: -1,
            mamba_host_src_index: -1,
            paged_cache: PagedCacheMatch::default(),
        }
    }
}
