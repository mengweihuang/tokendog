//! Main scheduler: orchestrates request lifecycle, prefix caching, and batch planning.

use std::collections::HashMap;

use crate::core::TokenContainer;
use crate::fsm::{State, Submitted};
use crate::resource::allocator::{PageAllocator, ReqPoolAllocator};
use crate::resource::hybrid_prefix_cache::{HybridPrefixCache, MambaChunkAllocator, MambaHostAllocator};
use crate::resource::kv_prefix_cache::KVPrefixCache;
use crate::resource::types::MatchIntent;

use super::execution_event::ExecutionEvent;
use super::execution_plan::ExecutionPlan;
use super::kv_cache_events::KvCacheEvent;
use super::operations::{
    CacheKind, DecodeOperation, FlatForwardOperation, ForwardOperation,
    WriteBackOperation,
};
use super::request::Request;
use super::request_spec::RequestSpec;
use super::types::{SchedulerConfig, SchedulerStats};

/// The main scheduler orchestrator.
///
/// Field declaration order matters for Drop ordering:
/// hybrid_prefix_cache must drop before kv_prefix_cache,
/// which must drop before mamba allocators.
pub struct Scheduler {
    config: SchedulerConfig,

    // Cache and tree components (drop order: hybrid → kv → allocators)
    // kv_prefix_cache is Box-allocated so its heap address is stable —
    // HybridPrefixCache holds a raw pointer to it that must not dangle.
    hybrid_prefix_cache: Option<HybridPrefixCache>,
    kv_prefix_cache: Box<KVPrefixCache>,

    // Allocators (must outlive cache components)
    device_allocator: PageAllocator,
    host_allocator: PageAllocator,
    mamba_allocator: Option<MambaChunkAllocator>,
    mamba_host_allocator: Option<MambaHostAllocator>,

    // Request tracking (drops before req_pool_allocator so ReqPoolIndex in
    // ForwardState can safely return slots to the still-live allocator)
    requests: HashMap<String, Request>,
    req_pool_allocator: ReqPoolAllocator,

    // KV events
    kv_events: Vec<KvCacheEvent>,

    // Stats
    stats: SchedulerStats,
}

impl Scheduler {
    /// Create a new Scheduler with the given configuration.
    pub fn new(config: SchedulerConfig) -> Self {
        let mut device_allocator = PageAllocator::new(config.page_size, config.device_total_pages);
        let mut host_allocator = PageAllocator::new(config.page_size, config.host_total_pages);

        // Box-allocate KVPrefixCache on the heap so its address is stable.
        // HybridPrefixCache holds a raw pointer to it that must not dangle.
        let mut kv_prefix_cache = Box::new(KVPrefixCache::new(
            &mut device_allocator,
            &mut host_allocator,
            config.enable_l3_storage,
            config.disable_prefix_cache,
        ));

        let mamba_allocator = if config.enable_mamba && config.mamba_pool_total_chunks > 0 {
            Some(MambaChunkAllocator::new(config.mamba_pool_total_chunks))
        } else {
            None
        };

        let mamba_host_allocator = if config.enable_mamba_l2 && config.mamba_l2_host_slots > 0 {
            Some(MambaHostAllocator::new(config.mamba_l2_host_slots))
        } else {
            None
        };

        let hybrid_prefix_cache = if config.enable_mamba || !config.paged_cache_groups.is_empty() {
            // Pass a raw pointer to the heap-allocated KVPrefixCache.
            // The Box guarantees the address is stable across moves.
            let kv_ptr: *mut KVPrefixCache = &mut *kv_prefix_cache;
            let mut hpc = HybridPrefixCache::new(
                kv_ptr,
                mamba_allocator,
                config.mamba_cache_chunk_size,
                mamba_host_allocator,
            );

            // Register paged-cache groups
            for group_config in &config.paged_cache_groups {
                use crate::resource::allocator::paged_cache_group::PagedCacheGroupAllocator;
                hpc.register_paged_cache_group(PagedCacheGroupAllocator::new(group_config.clone()));
            }

            // Enable paged-cache adjunct if configured
            if let Some(ref adjunct) = config.prefix_cache_adjunct {
                use std::collections::HashMap;
                hpc.enable_paged_cache_adjunct(
                    adjunct.required_groups.clone(),
                    HashMap::new(),
                    crate::resource::allocator::paged_cache_group::StateRestorePolicy::SnapshotRequired,
                );
            }

            Some(hpc)
        } else {
            None
        };

        let req_pool_allocator = ReqPoolAllocator::new(config.max_batch_size + 1);

        Self {
            config,
            hybrid_prefix_cache,
            kv_prefix_cache,
            device_allocator,
            host_allocator,
            mamba_allocator: None,
            mamba_host_allocator: None,
            req_pool_allocator,
            requests: HashMap::new(),
            kv_events: Vec::new(),
            stats: SchedulerStats::default(),
        }
    }

    /// Submit new requests to the scheduler.
    pub fn submit_requests(&mut self, request_specs: &[RequestSpec]) {
        for spec in request_specs {
            let tc = TokenContainer::new(spec.tokens.clone());
            let request = Request {
                id: spec.request_id.clone(),
                token_container: tc,
                state: State::Submitted(Submitted {
                    token_container: TokenContainer::new(spec.tokens.clone()),
                    page_size: self.config.page_size,
                }),
            };
            self.requests.insert(spec.request_id.clone(), request);
        }
    }

    /// Compute rolling SHA-256 hashes for input tokens.
    pub fn calc_rolling_hash(&self, input_tokens: &[i32], _apply_match: bool) -> Vec<String> {
        let page_size = self.config.page_size as usize;
        let mut hashes = Vec::new();
        for chunk in input_tokens.chunks(page_size) {
            let prior = hashes.last().map(|s: &String| s.as_str()).unwrap_or("");
            hashes.push(super::page_hasher::hash_page(chunk, prior));
        }
        hashes
    }

    /// Generate the next execution plan.
    pub fn next_execution_plan(&mut self) -> ExecutionPlan {
        let mut plan = ExecutionPlan::default();

        // Process drained/retracting writebacks
        let write_backs = self.new_write_back_operation();
        plan.write_backs = write_backs;

        // Collect candidates for forward scheduling
        let candidates: Vec<String> = self.requests
            .iter()
            .filter(|(_, r)| {
                matches!(r.state,
                    State::Submitted(_) | State::PrefetchDone(_) |
                    State::Prefilling(_) | State::PrefillDone(_) |
                    State::Decoding(_) | State::Retracted(_)
                )
            })
            .map(|(id, _)| id.clone())
            .collect();

        if candidates.is_empty() {
            return plan;
        }

        // Sort by priority: Prefilling > Submitted/PrefetchDone > Decoding/PrefillDone > Retracted
        let mut sorted = candidates;
        sorted.sort_by(|a, b| {
            let pa = self.request_priority(a);
            let pb = self.request_priority(b);
            pa.cmp(&pb).then_with(|| a.cmp(b)) // tiebreak on request_id
        });

        // Schedule forward operations
        let mut forward_ops: Vec<ForwardOperation> = Vec::new();
        let mut token_budget = self.config.max_scheduled_tokens;
        let mut batch_size = 0;

        for rid in &sorted {
            if batch_size >= self.config.max_batch_size || token_budget <= 0 {
                break;
            }

            let tokens_needed = match self.requests.get(rid) {
                Some(r) => match &r.state {
                    State::Submitted(s) => {
                        let remaining = s.token_container.size() as i32;
                        remaining.min(token_budget)
                    }
                    State::PrefillDone(_) => {
                        self.config.decode_input_tokens
                    }
                    State::Decoding(_d) => {
                        self.config.decode_input_tokens
                    }
                    State::Prefilling(p) => {
                        let remaining = p.base.base.token_container.size() as i32 - p.window_begin - p.window_size;
                        remaining.min(token_budget)
                    }
                    State::Retracted(_) => {
                        // Recovery: need 1 decode token
                        self.config.decode_input_tokens
                    }
                    _ => continue,
                },
                None => continue,
            };

            if tokens_needed <= 0 || tokens_needed > token_budget {
                continue;
            }

            // Match against prefix cache (optionally augmented by hybrid cache)
            let tokens = self.get_request_tokens(rid);
            let _match_result = if let Some(ref mut hybrid) = self.hybrid_prefix_cache {
                hybrid.match_tokens(&tokens, MatchIntent::PrefixReuse)
            } else {
                self.kv_prefix_cache.match_tokens(&tokens, MatchIntent::PrefixReuse)
            };

            // TODO: Full scheduling logic — capacity check, FSM event creation, operation generation
            // For now, create a placeholder decode operation
            forward_ops.push(ForwardOperation::Decode(DecodeOperation {
                request_id: rid.clone(),
                decode_input_id: tokens.last().copied().unwrap_or(0),
                occupied_pages: Vec::new(),
                page_size: self.config.page_size,
                hist_token_len: tokens.len() as i32,
                token_count: 1,
                paged_cache_pages: HashMap::new(),
                paged_cache_page_base_offsets: HashMap::new(),
                is_retract_recovery: false,
            }));

            token_budget -= tokens_needed;
            batch_size += 1;
        }

        if !forward_ops.is_empty() {
            plan.forward.push(FlatForwardOperation::from_ops(forward_ops));
        }

        plan
    }

    /// Advance the scheduler state with an execution event.
    pub fn advance(&mut self, event: &ExecutionEvent) {
        match event {
            ExecutionEvent::WriteBackDone { request_id } => {
                if let Some(req) = self.requests.get_mut(request_id) {
                    req.state = State::Finished;
                }
            }
            ExecutionEvent::Finish { request_id } => {
                if let Some(req) = self.requests.get_mut(request_id) {
                    req.state = State::Draining(
                        crate::fsm::Draining {
                            pages_to_transfer: Vec::new(),
                        }
                    );
                }
            }
            ExecutionEvent::Abort { request_id, .. } => {
                if let Some(req) = self.requests.get_mut(request_id) {
                    req.state = State::Finished;
                    self.stats.abort_count += 1;
                }
            }
            _ => {}
        }
    }

    /// Drain accumulated KV cache events.
    pub fn drain_kv_events(&mut self) -> Vec<KvCacheEvent> {
        std::mem::take(&mut self.kv_events)
    }

    // -----------------------------------------------------------------------
    // Query methods
    // -----------------------------------------------------------------------

    /// Number of waiting (not yet active) requests.
    pub fn waiting_size(&self) -> usize {
        self.requests.values().filter(|r| matches!(r.state, State::Submitted(_))).count()
    }

    /// Number of actively decoding requests.
    pub fn decoding_size(&self) -> usize {
        self.requests.values().filter(|r| matches!(r.state, State::Decoding(_))).count()
    }

    /// Number of retracted requests.
    pub fn retracted_size(&self) -> usize {
        self.requests.values().filter(|r| matches!(r.state, State::Retracted(_))).count()
    }

    /// Number of available device KV pages.
    pub fn available_kv_pages(&self) -> usize {
        self.device_allocator.available_pages() as usize
    }

    /// Number of active (in-use) KV pages.
    pub fn active_kv_pages(&self) -> usize {
        self.device_allocator.total_pages() as usize - self.device_allocator.available_pages() as usize
    }

    /// Number of requests currently in prefill.
    pub fn prefill_size(&self) -> usize {
        self.requests.values().filter(|r| matches!(r.state, State::Prefilling(_))).count()
    }

    /// Get the token count for a specific request.
    pub fn get_request_token_size(&self, id: &str) -> i32 {
        self.requests.get(id).map(|r| r.token_size()).unwrap_or(0)
    }

    /// Paged-cache group IDs.
    pub fn paged_cache_group_ids(&self) -> Vec<String> {
        self.hybrid_prefix_cache.as_ref()
            .map(|h| h.paged_cache_group_ids())
            .unwrap_or_default()
    }

    /// Paged-cache group total pages.
    pub fn paged_cache_group_total_pages(&self, group_id: &str) -> i32 {
        self.hybrid_prefix_cache.as_ref()
            .and_then(|h| h.paged_cache_group_total_pages(group_id))
            .unwrap_or(0)
    }

    /// Paged-cache group available pages.
    pub fn paged_cache_group_available_pages(&self, group_id: &str) -> i32 {
        self.hybrid_prefix_cache.as_ref()
            .and_then(|h| h.paged_cache_group_available_pages(group_id))
            .unwrap_or(0)
    }

    /// Paged-cache group failed alloc count.
    pub fn paged_cache_group_failed_alloc_count(&self, group_id: &str) -> i64 {
        self.hybrid_prefix_cache.as_ref()
            .and_then(|h| h.paged_cache_group_failed_alloc_count(group_id))
            .unwrap_or(0)
    }

    /// Get request's paged-cache page IDs.
    pub fn get_request_paged_cache_page_ids(&self, request_id: &str, group_id: &str) -> Vec<i32> {
        self.hybrid_prefix_cache.as_ref()
            .map(|h| h.get_request_paged_cache_page_ids(request_id, group_id))
            .unwrap_or_default()
    }

    /// Get request's paged-cache base logical page.
    pub fn get_request_paged_cache_base_logical_page(&self, request_id: &str, group_id: &str) -> i32 {
        self.hybrid_prefix_cache.as_ref()
            .map(|h| h.get_request_paged_cache_base_logical_page(request_id, group_id))
            .unwrap_or(0)
    }

    /// Scheduler statistics.
    pub fn stats(&self) -> &SchedulerStats {
        &self.stats
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn request_priority(&self, id: &str) -> i32 {
        match self.requests.get(id).map(|r| &r.state) {
            Some(State::Prefilling(_)) => 0,   // Highest priority
            Some(State::Submitted(_)) => 1,
            Some(State::PrefetchDone(_)) => 1,
            Some(State::Decoding(_)) => 2,
            Some(State::PrefillDone(_)) => 2,
            Some(State::Retracted(_)) => 3,     // Lowest priority
            _ => 99,
        }
    }

    fn get_request_tokens(&self, id: &str) -> Vec<i32> {
        self.requests.get(id)
            .map(|r| r.token_container.tokens().to_vec())
            .unwrap_or_default()
    }

    fn new_write_back_operation(&mut self) -> Vec<WriteBackOperation> {
        let mut ops = Vec::new();
        let draining_ids: Vec<String> = self.requests
            .iter()
            .filter(|(_, r)| matches!(r.state, State::Draining(_)))
            .map(|(id, _)| id.clone())
            .collect();

        for rid in draining_ids {
            ops.push(WriteBackOperation {
                request_id: rid.clone(),
                device_pages: Vec::new(),
                host_pages: Vec::new(),
                is_retract: false,
                kind: CacheKind::KV,
            });
            if let Some(req) = self.requests.get_mut(&rid) {
                req.state = State::WritingBack(
                    crate::fsm::WritingBack {
                        pages_to_transfer: Vec::new(),
                        is_retract: false,
                    }
                );
            }
        }
        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_scheduler() {
        let config = SchedulerConfig::default();
        let sched = Scheduler::new(config);
        assert_eq!(sched.waiting_size(), 0);
        assert_eq!(sched.decoding_size(), 0);
    }

    #[test]
    fn test_submit_request() {
        let mut config = SchedulerConfig::default();
        config.page_size = 16;
        config.device_total_pages = 1024;
        let mut sched = Scheduler::new(config);

        sched.submit_requests(&[RequestSpec {
            request_id: "req-1".to_string(),
            tokens: vec![1, 2, 3, 4, 5, 6, 7, 8],
            rolling_hashes: vec![],
            storage_hit_pages: 0,
        }]);

        assert_eq!(sched.waiting_size(), 1);
    }

    #[test]
    fn test_next_execution_plan() {
        let mut config = SchedulerConfig::default();
        config.page_size = 4;
        config.device_total_pages = 1024;
        config.max_scheduled_tokens = 64;
        config.max_batch_size = 8;
        let mut sched = Scheduler::new(config);

        sched.submit_requests(&[RequestSpec {
            request_id: "req-1".to_string(),
            tokens: vec![1, 2, 3, 4, 5, 6, 7, 8],
            rolling_hashes: vec![],
            storage_hit_pages: 0,
        }]);

        let plan = sched.next_execution_plan();
        // Should produce a forward operation
        assert!(!plan.forward.is_empty());
    }

    #[test]
    fn test_calc_rolling_hash() {
        let config = SchedulerConfig::default();
        let sched = Scheduler::new(config);
        let hashes = sched.calc_rolling_hash(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16], false);
        assert_eq!(hashes.len(), 1);
    }

    #[test]
    fn test_drain_kv_events() {
        let config = SchedulerConfig::default();
        let mut sched = Scheduler::new(config);
        let events = sched.drain_kv_events();
        assert!(events.is_empty());
    }
}
