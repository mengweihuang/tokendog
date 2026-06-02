//! Radix tree node and the Tree arena.
//!
//! Uses `slotmap::SlotMap` to avoid self-referential pointer issues.
//! NodeKey is a stable generational index that can be freely copied.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use slotmap::SlotMap;

use crate::resource::types::NodeKey;

use super::mamba_slot::MambaSlot;
use super::node_resource::NodeResource;
use super::paged_cache_snapshot::PagedCacheSnapshot;

/// A token vector hasher matching the C++ TokenVecHash.
#[derive(Default)]
pub struct TokenVecHash;

impl std::hash::BuildHasher for TokenVecHash {
    type Hasher = TokenVecHasher;

    fn build_hasher(&self) -> Self::Hasher {
        TokenVecHasher(0)
    }
}

pub struct TokenVecHasher(u64);

impl std::hash::Hasher for TokenVecHasher {
    fn write(&mut self, bytes: &[u8]) {
        // Convert bytes to i32 tokens (groups of 4)
        for chunk in bytes.chunks(4) {
            let mut buf = [0u8; 4];
            buf[..chunk.len()].copy_from_slice(chunk);
            let token = i32::from_le_bytes(buf);
            self.write_i32(token);
        }
    }

    fn write_i32(&mut self, token: i32) {
        let mut hash = self.0;
        hash ^= token as u64;
        hash = hash.wrapping_add(0x9e3779b9);
        hash = hash.wrapping_add(hash << 6);
        hash = hash.wrapping_add(hash >> 2);
        self.0 = hash;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Global sequence counter for deterministic tiebreaking.
static NEXT_SEQ_ID: AtomicI64 = AtomicI64::new(0);

/// Arena-based tree of nodes.
pub struct Tree {
    pub nodes: SlotMap<NodeKey, TreeNode>,
    pub root: NodeKey,
}

impl Tree {
    /// Create a new empty tree with a root node.
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(TreeNode::new(vec![], Instant::now()));
        Self { nodes, root }
    }

    /// Get a reference to a node by key.
    pub fn get(&self, key: NodeKey) -> Option<&TreeNode> {
        self.nodes.get(key)
    }

    /// Get a mutable reference to a node by key.
    pub fn get_mut(&mut self, key: NodeKey) -> Option<&mut TreeNode> {
        self.nodes.get_mut(key)
    }

    /// Insert a new node and return its key.
    pub fn insert(&mut self, node: TreeNode) -> NodeKey {
        self.nodes.insert(node)
    }

    /// Remove a node from the tree.
    pub fn remove(&mut self, key: NodeKey) -> Option<TreeNode> {
        self.nodes.remove(key)
    }

    /// Check if a node exists.
    pub fn contains(&self, key: NodeKey) -> bool {
        self.nodes.contains_key(key)
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

/// A radix tree node.
///
/// Stores a contiguous token segment, a map of children keyed by their
/// first-page tokens, and optional attached resources (device/host pages,
/// Mamba slots, paged-cache snapshots).
pub struct TreeNode {
    /// Parent node key (None for root).
    pub parent: Option<NodeKey>,
    /// Children, keyed by first-page tokens.
    pub children: HashMap<Vec<i32>, NodeKey, TokenVecHash>,
    /// Token segment stored at this node.
    pub tokens: Vec<i32>,
    /// Cumulative depth from root in tokens.
    pub depth_in_tokens: usize,
    /// SHA-256 page hashes.
    pub page_hashes: Vec<String>,
    /// FNV-1a block hashes.
    pub block_hashes: Vec<u64>,
    /// Last access time (for LRU eviction).
    pub last_access_time: Instant,
    /// Monotonic sequence ID for deterministic tiebreaking.
    pub seq_id: i64,
    /// Whether persisted to storage.
    pub storage_persisted: bool,
    /// Device resource (KV pages on GPU).
    pub device_resource: Option<Box<NodeResource>>,
    /// Host resource (KV pages on CPU).
    pub host_resource: Option<Box<NodeResource>>,
    /// Mamba state slot (device).
    pub mamba_slot: Option<MambaSlot>,
    /// Mamba state slot (host).
    pub mamba_host_slot: Option<MambaSlot>,
    /// Paged-cache adjunct snapshot.
    pub paged_cache_snapshot: Option<PagedCacheSnapshot>,
}

impl TreeNode {
    /// Create a new tree node with the given tokens and access time.
    pub fn new(tokens: Vec<i32>, access_time: Instant) -> Self {
        let seq_id = NEXT_SEQ_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            parent: None,
            children: HashMap::with_hasher(TokenVecHash::default()),
            tokens,
            depth_in_tokens: 0,
            page_hashes: Vec::new(),
            block_hashes: Vec::new(),
            last_access_time: access_time,
            seq_id,
            storage_persisted: false,
            device_resource: None,
            host_resource: None,
            mamba_slot: None,
            mamba_host_slot: None,
            paged_cache_snapshot: None,
        }
    }

    /// Whether this is the root node.
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    /// Whether this node has device resource attached.
    pub fn on_device(&self) -> bool {
        self.device_resource.is_some()
    }

    /// Whether this node has host resource attached.
    pub fn on_host(&self) -> bool {
        self.host_resource.is_some()
    }

    /// Depth in pages (truncating division).
    pub fn depth_in_page(&self, page_size: i32) -> i32 {
        self.depth_in_tokens as i32 / page_size
    }

    /// Number of children.
    pub fn num_children(&self) -> usize {
        self.children.len()
    }

    /// Whether the node has a Mamba slot on device.
    pub fn has_mamba(&self) -> bool {
        self.mamba_slot.is_some()
    }

    /// Whether the node has a Mamba slot on host.
    pub fn has_mamba_on_host(&self) -> bool {
        self.mamba_host_slot.is_some()
    }

    /// Mamba device slot index.
    pub fn mamba_slot_index(&self) -> i32 {
        self.mamba_slot.as_ref().map(|s| s.index()).unwrap_or(-1)
    }

    /// Mamba host slot index.
    pub fn mamba_host_slot_index(&self) -> i32 {
        self.mamba_host_slot.as_ref().map(|s| s.index()).unwrap_or(-1)
    }

    /// Whether a paged-cache snapshot is attached.
    pub fn has_paged_cache_snapshot(&self) -> bool {
        self.paged_cache_snapshot.is_some()
    }

    /// Update the last access time.
    pub fn touch(&mut self, now: Instant) {
        self.last_access_time = now;
    }

    /// Add a child node, keyed by its first-page tokens.
    /// `parent_key` is the key of this node (the parent) in the tree.
    pub fn add_child(&mut self, key: Vec<i32>, child: NodeKey, _parent_key: NodeKey) {
        self.children.insert(key, child);
    }

    /// Remove a child by first-page key.
    pub fn remove_child(&mut self, key: &[i32]) -> Option<NodeKey> {
        self.children.remove(key)
    }

    /// Split this node into a prefix (kept in self prefix_node) and suffix (self).
    /// `prefix_pages` is the number of pages to keep in the prefix.
    pub fn split_self_into(&mut self, prefix: &mut TreeNode, prefix_pages: usize, page_size: i32) {
        let prefix_tokens = prefix_pages * page_size as usize;

        // Prefix gets the first prefix_tokens tokens
        prefix.tokens = self.tokens[..prefix_tokens].to_vec();
        prefix.depth_in_tokens = self.depth_in_tokens;

        // Copy hashes if present (guard against empty vectors from new nodes)
        if !self.page_hashes.is_empty() && prefix_pages <= self.page_hashes.len() {
            prefix.page_hashes = self.page_hashes[..prefix_pages].to_vec();
            self.page_hashes = self.page_hashes[prefix_pages..].to_vec();
        }
        if !self.block_hashes.is_empty() && prefix_pages <= self.block_hashes.len() {
            prefix.block_hashes = self.block_hashes[..prefix_pages].to_vec();
            self.block_hashes = self.block_hashes[prefix_pages..].to_vec();
        }

        // Self keeps the suffix
        self.tokens = self.tokens[prefix_tokens..].to_vec();
        self.depth_in_tokens += prefix_tokens;
    }

    /// Detach device resource, returning the NodeResource.
    pub fn detach_device_resource(&mut self) -> Option<Box<NodeResource>> {
        self.device_resource.take()
    }

    /// Detach host resource, returning the NodeResource.
    pub fn detach_host_resource(&mut self) -> Option<Box<NodeResource>> {
        self.host_resource.take()
    }

    /// Attach device resource.
    pub fn attach_device_resource(&mut self, resource: Box<NodeResource>) {
        self.device_resource = Some(resource);
    }

    /// Attach host resource.
    pub fn attach_host_resource(&mut self, resource: Box<NodeResource>) {
        self.host_resource = Some(resource);
    }
}
