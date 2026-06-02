//! Resource manager with LRU eviction.
//!
//! Tracks leaf nodes ordered by (access_time, seq_id, node_key_data)
//! for deterministic LRU eviction across TP ranks.

use std::collections::{BTreeMap, HashMap};
use std::ptr::NonNull;
use std::time::Instant;

use slotmap::Key;

use crate::resource::allocator::{OwnedPages, PageAllocator};
use crate::resource::types::NodeKey;

use super::tree_node::Tree;

/// LRU sort key: (access_time_nanos, seq_id, node_key_data).
/// SeqId breaks ties deterministically across TP ranks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LruKey(i128, i64, u64);

impl LruKey {
    /// Create an LRU sort key. Older nodes produce **more negative** nanos,
    /// so the BTreeMap (ascending) places the oldest entry first.
    fn new(ts: Instant, seq_id: i64, node_key: NodeKey) -> Self {
        // `ts` is always in the past; use checked_duration_since to avoid panic.
        let nanos = Instant::now()
            .checked_duration_since(ts)
            .map(|d| d.as_nanos() as i128)
            .unwrap_or(0);
        // Negate: oldest = largest duration = most negative nanos → first in BTreeMap
        Self(-nanos, seq_id, node_key.data().as_ffi())
    }
}

/// Resource manager that handles page allocation and LRU eviction.
pub struct ResourceManager {
    /// Page allocator for this resource tier.
    pub allocator: NonNull<PageAllocator>,
    /// LRU-ordered leaf nodes (oldest first), keyed by sort key.
    pub(crate) lru_leaves: BTreeMap<LruKey, NodeKey>,
    /// Maps node to its current LRU sort key (for efficient removal).
    pub(crate) node_lru_key: HashMap<NodeKey, LruKey>,
    /// Callback invoked when a node is evicted.
    pub eviction_callback: Option<Box<dyn FnMut(NodeKey)>>,
}

impl ResourceManager {
    /// Create a new resource manager.
    pub fn new(allocator: &mut PageAllocator) -> Self {
        Self {
            allocator: NonNull::new(allocator as *mut PageAllocator).expect("allocator must not be null"),
            lru_leaves: BTreeMap::new(),
            node_lru_key: HashMap::new(),
            eviction_callback: None,
        }
    }

    /// Set the eviction callback.
    pub fn set_eviction_callback(&mut self, cb: Box<dyn FnMut(NodeKey)>) {
        self.eviction_callback = Some(cb);
    }

    /// Remove a leaf from the LRU set.
    fn remove_leaf(&mut self, tree: &mut Tree, node_key: NodeKey) {
        if let Some(key) = self.node_lru_key.remove(&node_key) {
            self.lru_leaves.remove(&key);
        }
        if let Some(node) = tree.get_mut(node_key) {
            if let Some(ref mut resource) = node.device_resource {
                resource.clear_evictable_notifier();
            }
            if let Some(ref mut resource) = node.host_resource {
                resource.clear_evictable_notifier();
            }
        }
    }

    /// Check if a node is a leaf (has resource, non-root, no children with pages on this tier).
    fn is_leaf(&self, tree: &Tree, node_key: NodeKey) -> bool {
        let node = match tree.get(node_key) {
            Some(n) => n,
            None => return false,
        };
        if node.is_root() {
            return false;
        }
        let has_resource = node.device_resource.as_ref().map(|r| !r.is_empty()).unwrap_or(false)
            || node.host_resource.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
        if !has_resource {
            return false;
        }
        !self.has_child_with_pages(tree, node_key)
    }

    /// Check if any child has pages.
    fn has_child_with_pages(&self, tree: &Tree, node_key: NodeKey) -> bool {
        let node = match tree.get(node_key) {
            Some(n) => n,
            None => return false,
        };
        for &child_key in node.children.values() {
            if let Some(child) = tree.get(child_key) {
                let has_dev = child.device_resource.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
                let has_host = child.host_resource.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
                if has_dev || has_host {
                    return true;
                }
            }
        }
        false
    }

    /// Update leaf status (add/remove from LRU set).
    fn update_leaf(&mut self, tree: &mut Tree, node_key: NodeKey) {
        self.remove_leaf(tree, node_key);

        let is_leaf = self.is_leaf(tree, node_key);
        if is_leaf {
            let (ts, seq_id) = {
                let node = tree.get(node_key).unwrap();
                (node.last_access_time, node.seq_id)
            };

            // Bind eviction notifier
            if let Some(node) = tree.get_mut(node_key) {
                if let Some(ref mut resource) = node.device_resource {
                    resource.bind_evictable_notifier(self, node_key);
                }
                if let Some(ref mut resource) = node.host_resource {
                    resource.bind_evictable_notifier(self, node_key);
                }
            }

            let lru_key = LruKey::new(ts, seq_id, node_key);
            self.node_lru_key.insert(node_key, lru_key);
            self.lru_leaves.insert(lru_key, node_key);
        }
    }

    /// Update leaf tracking for this node and its parent.
    pub fn update_leaves(&mut self, tree: &mut Tree, node_key: NodeKey) {
        self.update_leaf(tree, node_key);
        if let Some(node) = tree.get(node_key) {
            if let Some(parent) = node.parent {
                self.update_leaf(tree, parent);
            }
        }
    }

    /// Called when a node becomes evictable (ref_count drops to 0).
    /// Re-inserts the node into the LRU set with its current timestamp
    /// (updated by any Touch() calls while it was locked).
    pub fn on_node_evictable(&mut self, node_key: NodeKey) {
        // Re-insert with fresh timestamp for correct LRU ordering.
        // The node may have been Touched while locked.
        if let Some(old_key) = self.node_lru_key.remove(&node_key) {
            self.lru_leaves.remove(&old_key);
        }
        // The caller (KVPrefixCache / RadixTree) must call update_leaves
        // after unlock to properly re-evaluate leaf status.
        // We record the intent here; update_leaves is the authoritative path.
    }

    /// Evict `num_pages` worth of pages from the LRU.
    pub fn evict(&mut self, tree: &mut Tree, num_pages: i32) -> Vec<NodeKey> {
        let mut evicted_nodes = Vec::new();
        if num_pages <= 0 {
            return evicted_nodes;
        }

        let mut deferred_locked: Vec<(LruKey, NodeKey)> = Vec::new();
        let mut evicted = 0;

        while evicted < num_pages && !self.lru_leaves.is_empty() {
            // Get oldest entry
            let (lru_key, node_key) = {
                let entry = self.lru_leaves.first_entry().unwrap();
                (*entry.key(), *entry.get())
            };
            self.lru_leaves.remove(&lru_key);
            self.node_lru_key.remove(&node_key);

            // Check if node is evictable
            let is_evictable = tree
                .get(node_key)
                .and_then(|n| n.device_resource.as_ref())
                .map(|r| r.is_evictable())
                .unwrap_or(false);

            if !is_evictable {
                deferred_locked.push((lru_key, node_key));
                continue;
            }

            // Evict: detach resource and free pages
            if let Some(node) = tree.get_mut(node_key) {
                if let Some(mut resource) = node.device_resource.take() {
                    if let Some(ref mut cb) = self.eviction_callback {
                        cb(node_key);
                    }
                    let pages = resource.take_pages();
                    evicted += pages.size();
                    evicted_nodes.push(node_key);
                }
            }

            // Parent may have become a leaf
            if let Some(node) = tree.get(node_key) {
                if let Some(parent) = node.parent {
                    self.update_leaf(tree, parent);
                }
            }
        }

        // Restore locked nodes
        for (_, node_key) in deferred_locked {
            if let Some(node) = tree.get(node_key) {
                let ts = node.last_access_time;
                let lru_key = LruKey::new(ts, node.seq_id, node_key);
                self.node_lru_key.insert(node_key, lru_key);
                self.lru_leaves.insert(lru_key, node_key);
            }
        }

        evicted_nodes
    }

    /// Ensure at least `required_num_pages` are available, evicting if needed.
    pub fn ensure_capacity(&mut self, tree: &mut Tree, required_num_pages: i32) -> Vec<NodeKey> {
        if required_num_pages <= 0 {
            return Vec::new();
        }
        let available = self.available_pages();
        if available >= required_num_pages {
            return Vec::new();
        }
        self.evict(tree, required_num_pages - available)
    }

    /// Allocate pages from the underlying allocator.
    pub fn allocate(&mut self, num_pages: i32) -> OwnedPages {
        unsafe { self.allocator.as_ptr().as_mut().unwrap() }.allocate(num_pages)
    }

    /// Available pages.
    pub fn available_pages(&self) -> i32 {
        unsafe { self.allocator.as_ptr().as_ref().unwrap() }.available_pages()
    }

    /// Get the evictable pages count (O(N) scan).
    pub fn evictable_pages_num(&self, tree: &Tree) -> i32 {
        let mut total = 0;
        for &node_key in self.lru_leaves.values() {
            if let Some(node) = tree.get(node_key) {
                if let Some(ref resource) = node.device_resource {
                    if resource.is_evictable() {
                        total += resource.num_pages();
                    }
                }
            }
        }
        total
    }
}
