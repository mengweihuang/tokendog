//! Scheduler configuration and statistics types.

use crate::resource::allocator::paged_cache_group::PagedCacheGroupConfig;
use crate::resource::types::{DisaggregationMode, Role};

/// Prefix-cache adjunct spec for paged-cache groups.
#[derive(Debug, Clone)]
pub struct PrefixCacheAdjunctSpec {
    pub required_groups: Vec<String>,
}

/// Scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Page size in tokens.
    pub page_size: i32,
    /// Device (GPU) allocator config.
    pub device_total_pages: i32,
    /// Host (CPU) allocator config.
    pub host_total_pages: i32,
    /// Paged-cache group configs.
    pub paged_cache_groups: Vec<PagedCacheGroupConfig>,
    /// Prefix cache adjunct spec (optional).
    pub prefix_cache_adjunct: Option<PrefixCacheAdjunctSpec>,
    /// Maximum scheduled tokens per batch.
    pub max_scheduled_tokens: i32,
    /// Maximum batch size.
    pub max_batch_size: i32,
    /// Decode input tokens (default 1).
    pub decode_input_tokens: i32,
    /// Whether L2 cache is disabled.
    pub disable_l2_cache: bool,
    /// Whether L3 storage is enabled.
    pub enable_l3_storage: bool,
    /// Prefetch threshold (num pages).
    pub prefetch_threshold: i32,
    /// Whether KV cache events are enabled.
    pub enable_kv_cache_events: bool,
    /// Whether mixed prefill/decode batching is enabled.
    pub enable_mixed_prefill_decode: bool,
    /// Pages reserved for retracted or running requests.
    pub num_pages_reserved_for_retracted_or_running: i32,
    /// Scheduler role (P, D, or Fused).
    pub role: Role,
    /// Whether prefix cache is disabled.
    pub disable_prefix_cache: bool,
    /// Whether Mamba cache is enabled.
    pub enable_mamba: bool,
    /// Mamba cache chunk size.
    pub mamba_cache_chunk_size: i32,
    /// Mamba pool total chunks.
    pub mamba_pool_total_chunks: i32,
    /// Whether Mamba L2 is enabled.
    pub enable_mamba_l2: bool,
    /// Mamba L2 host slots.
    pub mamba_l2_host_slots: i32,
    /// Disaggregation mode.
    pub disaggregation_mode: DisaggregationMode,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            page_size: 16,
            device_total_pages: 1024,
            host_total_pages: 0,
            paged_cache_groups: Vec::new(),
            prefix_cache_adjunct: None,
            max_scheduled_tokens: 4096,
            max_batch_size: 32,
            decode_input_tokens: 1,
            disable_l2_cache: false,
            enable_l3_storage: false,
            prefetch_threshold: 4,
            enable_kv_cache_events: false,
            enable_mixed_prefill_decode: false,
            num_pages_reserved_for_retracted_or_running: 0,
            role: Role::Fused,
            disable_prefix_cache: false,
            enable_mamba: false,
            mamba_cache_chunk_size: 64,
            mamba_pool_total_chunks: 0,
            enable_mamba_l2: false,
            mamba_l2_host_slots: 0,
            disaggregation_mode: DisaggregationMode::None,
        }
    }
}

/// Scheduler statistics.
#[derive(Debug, Clone, Default)]
pub struct SchedulerStats {
    pub total_batches: i64,
    pub mixed_batches: i64,
    pub retract_count: i64,
    pub abort_count: i64,
    pub schedule_latency_count: i64,
    pub schedule_latency_sum_us: i64,
    pub schedule_latency_max_us: i64,
    pub prefix_cache_hit_tokens: i64,
    pub prefix_cache_req_tokens: i64,
    pub pending_queue_size: i64,
    pub plan_queue_size: i64,
    pub event_queue_size: i64,
    pub active_requests: i64,
}
