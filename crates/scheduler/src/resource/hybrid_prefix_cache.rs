//! Hybrid prefix cache: KV prefix cache + Mamba cache + paged-cache groups.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::resource::allocator::paged_cache_group::{
    PagedCacheGroupAllocator, PagedCacheGroupTable, StateRestorePolicy,
};
use crate::resource::kv_prefix_cache::KVPrefixCache;
use crate::resource::types::{MatchIntent, MatchResult, NodeKey};

/// Pair of (device_page, host_page) for device↔host transfers.
#[derive(Debug, Clone)]
pub struct TransferPair {
    pub device_page: i32,
    pub host_page: i32,
}

/// Mamba chunk allocator (min-heap based).
pub struct MambaChunkAllocator {
    free_list: std::collections::BinaryHeap<std::cmp::Reverse<i32>>,
    total_slots: i32,
}

impl MambaChunkAllocator {
    pub fn new(total_slots: i32) -> Self {
        let mut free_list = std::collections::BinaryHeap::new();
        for i in 0..total_slots {
            free_list.push(std::cmp::Reverse(i));
        }
        Self {
            free_list,
            total_slots,
        }
    }

    pub fn allocate(&mut self) -> Option<i32> {
        self.free_list.pop().map(|r| r.0)
    }

    pub fn free(&mut self, slot: i32) {
        self.free_list.push(std::cmp::Reverse(slot));
    }

    pub fn available(&self) -> i32 {
        self.free_list.len() as i32
    }
}

/// Mamba host allocator.
pub type MambaHostAllocator = MambaChunkAllocator;

/// Hybrid prefix cache combining KV, Mamba, and paged-cache.
pub struct HybridPrefixCache {
    kv_prefix_cache: *mut KVPrefixCache,
    mamba_allocator: Option<MambaChunkAllocator>,
    mamba_host_allocator: Option<MambaHostAllocator>,
    mamba_cache_chunk_size: i32,
    mamba_host_nodes: HashSet<NodeKey>,
    paged_cache_allocators: BTreeMap<String, PagedCacheGroupAllocator>,
    request_paged_cache_tables: HashMap<String, BTreeMap<String, PagedCacheGroupTable>>,
    paged_cache_history_alignment_tokens: i32,
    paged_cache_required_groups: Vec<String>,
    paged_cache_sliding_window_per_group: HashMap<String, i32>,
    paged_cache_history_groups: Vec<String>,
    paged_cache_state_groups: Vec<String>,
    paged_cache_history_group_set: HashSet<String>,
    paged_cache_state_group_set: HashSet<String>,
    paged_cache_snapshot_nodes: HashSet<NodeKey>,
}

impl HybridPrefixCache {
    /// Create a new HybridPrefixCache.
    ///
    /// # Safety
    ///
    /// `kv_prefix_cache` must point to a valid, heap-stable KVPrefixCache
    /// that outlives this HybridPrefixCache.
    pub fn new(
        kv_prefix_cache: *mut KVPrefixCache,
        mamba_allocator: Option<MambaChunkAllocator>,
        mamba_cache_chunk_size: i32,
        mamba_host_allocator: Option<MambaHostAllocator>,
    ) -> Self {
        Self {
            kv_prefix_cache,
            mamba_allocator,
            mamba_host_allocator,
            mamba_cache_chunk_size,
            mamba_host_nodes: HashSet::new(),
            paged_cache_allocators: BTreeMap::new(),
            request_paged_cache_tables: HashMap::new(),
            paged_cache_history_alignment_tokens: 0,
            paged_cache_required_groups: Vec::new(),
            paged_cache_sliding_window_per_group: HashMap::new(),
            paged_cache_history_groups: Vec::new(),
            paged_cache_state_groups: Vec::new(),
            paged_cache_history_group_set: HashSet::new(),
            paged_cache_state_group_set: HashSet::new(),
            paged_cache_snapshot_nodes: HashSet::new(),
        }
    }

    fn kv(&self) -> &KVPrefixCache {
        unsafe { &*self.kv_prefix_cache }
    }

    fn kv_mut(&mut self) -> &mut KVPrefixCache {
        unsafe { &mut *self.kv_prefix_cache }
    }

    pub fn has_mamba_adjunct(&self) -> bool {
        self.mamba_allocator.is_some()
    }

    pub fn has_paged_cache_adjunct(&self) -> bool {
        self.paged_cache_history_alignment_tokens > 0
    }

    pub fn available_slots(&self) -> i32 {
        self.mamba_allocator.as_ref().map(|a| a.available()).unwrap_or(0)
    }

    pub fn mamba_cache_chunk_size(&self) -> i32 {
        self.mamba_cache_chunk_size
    }

    /// Match with Mamba and paged-cache augmentation.
    pub fn match_tokens(&mut self, token_ids: &[i32], intent: MatchIntent) -> MatchResult {
        self.kv_mut().match_tokens(token_ids, intent)
    }

    /// Register a paged-cache group.
    pub fn register_paged_cache_group(&mut self, allocator: PagedCacheGroupAllocator) {
        let group_id = allocator.config().group_id.clone();
        self.paged_cache_allocators.insert(group_id, allocator);
    }

    /// Enable paged-cache adjunct.
    pub fn enable_paged_cache_adjunct(
        &mut self,
        required_groups: Vec<String>,
        sliding_window_per_group: HashMap<String, i32>,
        _policy: StateRestorePolicy,
    ) {
        self.paged_cache_required_groups = required_groups.clone();
        self.paged_cache_sliding_window_per_group = sliding_window_per_group;

        // Compute history alignment (LCM of raw_tokens_per_page for History groups)
        let mut alignment = 1;
        for gid in &required_groups {
            if let Some(alloc) = self.paged_cache_allocators.get(gid) {
                let rtp = alloc.config().raw_tokens_per_page();
                alignment = lcm(alignment, rtp);
            }
        }
        self.paged_cache_history_alignment_tokens = alignment;

        // Partition groups by family
        self.paged_cache_history_groups.clear();
        self.paged_cache_state_groups.clear();
        self.paged_cache_history_group_set.clear();
        self.paged_cache_state_group_set.clear();

        for gid in &required_groups {
            if let Some(alloc) = self.paged_cache_allocators.get(gid) {
                match alloc.config().family {
                    crate::resource::allocator::paged_cache_group::PagedCacheGroupFamily::History => {
                        self.paged_cache_history_groups.push(gid.clone());
                        self.paged_cache_history_group_set.insert(gid.clone());
                    }
                    crate::resource::allocator::paged_cache_group::PagedCacheGroupFamily::State => {
                        self.paged_cache_state_groups.push(gid.clone());
                        self.paged_cache_state_group_set.insert(gid.clone());
                    }
                }
            }
        }
    }

    /// Acquire pages for a request.
    pub fn acquire_for_request(
        &mut self,
        request_id: &str,
        _first_raw_position: i32,
        target_raw_tokens: i32,
        _paged_cache_hit: &crate::resource::types::PagedCacheMatch,
    ) {
        let tables = self
            .request_paged_cache_tables
            .entry(request_id.to_string())
            .or_default();

        for (gid, alloc) in &mut self.paged_cache_allocators {
            let table = tables.entry(gid.clone()).or_insert_with(|| {
                PagedCacheGroupTable::with_allocator(alloc)
            });
            table.acquire(target_raw_tokens);
        }
    }

    /// Release a request's pages.
    pub fn release_request(&mut self, request_id: &str) {
        if let Some(tables) = self.request_paged_cache_tables.remove(request_id) {
            for (gid, mut table) in tables {
                let _released = table.release_all();
                if let Some(alloc) = self.paged_cache_allocators.get_mut(&gid) {
                    alloc.deallocate(&_released);
                }
            }
        }
    }

    /// Paged cache group IDs.
    pub fn paged_cache_group_ids(&self) -> Vec<String> {
        self.paged_cache_allocators.keys().cloned().collect()
    }

    /// Group total pages.
    pub fn paged_cache_group_total_pages(&self, group_id: &str) -> Option<i32> {
        self.paged_cache_allocators.get(group_id).map(|a| a.total_pages())
    }

    /// Group available pages.
    pub fn paged_cache_group_available_pages(&self, group_id: &str) -> Option<i32> {
        self.paged_cache_allocators.get(group_id).map(|a| a.available_pages())
    }

    /// Group failed alloc count.
    pub fn paged_cache_group_failed_alloc_count(&self, group_id: &str) -> Option<i64> {
        self.paged_cache_allocators.get(group_id).map(|a| a.failed_alloc_count())
    }

    /// Get request's paged-cache page IDs.
    pub fn get_request_paged_cache_page_ids(
        &self,
        request_id: &str,
        group_id: &str,
    ) -> Vec<i32> {
        self.request_paged_cache_tables
            .get(request_id)
            .and_then(|t| t.get(group_id))
            .map(|t| t.page_ids().to_vec())
            .unwrap_or_default()
    }

    /// Get request's paged-cache base logical page.
    pub fn get_request_paged_cache_base_logical_page(
        &self,
        request_id: &str,
        group_id: &str,
    ) -> i32 {
        self.request_paged_cache_tables
            .get(request_id)
            .and_then(|t| t.get(group_id))
            .map(|t| t.base_logical_page())
            .unwrap_or(0)
    }

    pub fn mamba_allocator_mut(&mut self) -> Option<&mut MambaChunkAllocator> {
        self.mamba_allocator.as_mut()
    }

    pub fn mamba_host_allocator_mut(&mut self) -> Option<&mut MambaHostAllocator> {
        self.mamba_host_allocator.as_mut()
    }
}

fn gcd(a: i32, b: i32) -> i32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn lcm(a: i32, b: i32) -> i32 {
    if a == 0 || b == 0 { 0 } else { a / gcd(a, b) * b }
}
