//! KV prefix cache: radix tree with device/host resource managers.

use std::collections::HashSet;

use crate::resource::allocator::PageAllocator;
use crate::resource::radix_tree::RadixTree;
use crate::resource::radix_tree::ResourceManager;
use crate::resource::types::{InsertResult, MatchIntent, MatchResult};

/// KV prefix cache wrapping a RadixTree with Device and Host resource managers.
pub struct KVPrefixCache {
    tree: RadixTree,
    pub device: ResourceManager,
    pub host: ResourceManager,
    next_op_id: u32,
    enable_l3_storage: bool,
    disable_prefix_cache: bool,
    published_device_blocks: HashSet<u64>,
}

impl KVPrefixCache {
    /// Create a new KVPrefixCache.
    pub fn new(
        device_allocator: &mut PageAllocator,
        host_allocator: &mut PageAllocator,
        enable_l3_storage: bool,
        disable_prefix_cache: bool,
    ) -> Self {
        let page_size = device_allocator.page_size();
        Self {
            tree: RadixTree::new(page_size),
            device: ResourceManager::new(device_allocator),
            host: ResourceManager::new(host_allocator),
            next_op_id: 1,
            enable_l3_storage,
            disable_prefix_cache,
            published_device_blocks: HashSet::new(),
        }
    }

    /// Match tokens against the prefix cache.
    pub fn match_tokens(&mut self, token_ids: &[i32], _intent: MatchIntent) -> MatchResult {
        if self.disable_prefix_cache {
            return MatchResult::default();
        }
        let now = std::time::Instant::now();
        let walk = self.tree.walk_down_util_mismatch(token_ids, now, None);
        walk.match_result
    }

    /// Walk down matching token pages.
    pub fn match_pages(&mut self, _token_pages: &[&[i32]], _intent: MatchIntent) -> MatchResult {
        // Flatten pages and match
        let tokens: Vec<i32> = _token_pages.iter().flat_map(|p| p.iter()).copied().collect();
        self.match_tokens(&tokens, _intent)
    }

    /// Insert tokens with device pages.
    pub fn insert_device(
        &mut self,
        _token_ids: &[i32],
        _prefix_pages: &[i32],
        _allocator_pages: crate::resource::allocator::OwnedPages,
        _page_hashes: &[String],
    ) -> InsertResult {
        // Walk down, split as needed, attach device resource
        // TODO: full implementation
        InsertResult {
            last_node: self.tree.root(),
            inserted_num_pages: _allocator_pages.size(),
        }
    }

    /// Insert tokens with host pages.
    pub fn insert_host(
        &mut self,
        _token_ids: &[i32],
        _prefix_pages: &[i32],
        _allocator_pages: crate::resource::allocator::OwnedPages,
        _page_hashes: &[String],
    ) -> InsertResult {
        InsertResult {
            last_node: self.tree.root(),
            inserted_num_pages: _allocator_pages.size(),
        }
    }

    /// Allocate a new cache operation ID.
    pub fn allocate_cache_op_id(&mut self) -> u32 {
        let id = self.next_op_id;
        self.next_op_id += 1;
        id
    }

    /// Ensure device capacity by evicting if needed.
    pub fn ensure_capacity_device(&mut self, required: i32) -> bool {
        let available = self.device.available_pages();
        if available >= required {
            return true;
        }
        let evicted = self.device.evict(&mut self.tree.tree, required - available);
        !evicted.is_empty() || self.device.available_pages() >= required
    }

    /// Ensure host capacity by evicting if needed.
    pub fn ensure_capacity_host(&mut self, required: i32) -> bool {
        let available = self.host.available_pages();
        if available >= required {
            return true;
        }
        let evicted = self.host.evict(&mut self.tree.tree, required - available);
        !evicted.is_empty() || self.host.available_pages() >= required
    }

    /// Page size.
    pub fn page_size(&self) -> i32 {
        self.tree.page_size
    }

    /// Access the radix tree.
    pub fn radix_tree(&self) -> &RadixTree {
        &self.tree
    }

    pub fn radix_tree_mut(&mut self) -> &mut RadixTree {
        &mut self.tree
    }

    /// Device resource manager access.
    pub fn device_manager(&self) -> &ResourceManager {
        &self.device
    }

    pub fn device_manager_mut(&mut self) -> &mut ResourceManager {
        &mut self.device
    }

    /// Host resource manager access.
    pub fn host_manager(&self) -> &ResourceManager {
        &self.host
    }

    pub fn host_manager_mut(&mut self) -> &mut ResourceManager {
        &mut self.host
    }
}
