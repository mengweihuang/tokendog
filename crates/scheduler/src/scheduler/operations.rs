//! Operation types output by the scheduler.

use std::collections::HashMap;

/// A forward operation for the inference engine.
#[derive(Debug, Clone)]
pub enum ForwardOperation {
    Prefill(PrefillOperation),
    Decode(DecodeOperation),
}

/// Prefill operation: batch of prompt tokens to process.
#[derive(Debug, Clone)]
pub struct PrefillOperation {
    pub request_id: String,
    pub input_ids: Vec<i32>,
    pub shifted_input_ids: Vec<i32>,
    pub occupied_pages: Vec<i32>,
    pub page_size: i32,
    pub extend_prefix_len: i32,
    pub begin: i32,
    pub size: i32,
    pub token_count: i32,
    pub hist_token_len: i32,
    pub paged_cache_pages: HashMap<String, Vec<i32>>,
    pub paged_cache_page_base_offsets: HashMap<String, i32>,
    pub is_retract_recovery: bool,
}

/// Decode operation: single token generation step.
#[derive(Debug, Clone)]
pub struct DecodeOperation {
    pub request_id: String,
    pub decode_input_id: i32,
    pub occupied_pages: Vec<i32>,
    pub page_size: i32,
    pub hist_token_len: i32,
    pub token_count: i32,
    pub paged_cache_pages: HashMap<String, Vec<i32>>,
    pub paged_cache_page_base_offsets: HashMap<String, i32>,
    pub is_retract_recovery: bool,
}

/// Flattened forward operation for batched execution (SoA layout).
#[derive(Debug, Clone, Default)]
pub struct FlatForwardOperation {
    pub request_ids: Vec<String>,
    pub input_ids: Vec<i32>,
    pub shifted_input_ids: Vec<i32>,
    pub occupied_pages: Vec<Vec<i32>>,
    pub page_sizes: Vec<i32>,
    pub extend_prefix_lens: Vec<i32>,
    pub begins: Vec<i32>,
    pub sizes: Vec<i32>,
    pub token_counts: Vec<i32>,
    pub decode_input_ids: Vec<i32>,
    pub hist_token_lens: Vec<i32>,
    pub is_prefill: Vec<bool>,
    pub is_retract_recovery: Vec<bool>,
    pub paged_cache_pages: Vec<HashMap<String, Vec<i32>>>,
    pub paged_cache_page_base_offsets: Vec<HashMap<String, i32>>,
    pub paged_cache_block_tables: Vec<Vec<Vec<i32>>>,
}

impl FlatForwardOperation {
    pub fn from_ops(ops: Vec<ForwardOperation>) -> Self {
        let mut flat = Self::default();
        for op in ops {
            match op {
                ForwardOperation::Prefill(p) => {
                    flat.request_ids.push(p.request_id);
                    flat.input_ids.extend(p.input_ids);
                    flat.shifted_input_ids.extend(p.shifted_input_ids);
                    flat.occupied_pages.push(p.occupied_pages);
                    flat.page_sizes.push(p.page_size);
                    flat.extend_prefix_lens.push(p.extend_prefix_len);
                    flat.begins.push(p.begin);
                    flat.sizes.push(p.size);
                    flat.token_counts.push(p.token_count);
                    flat.decode_input_ids.push(-1);
                    flat.hist_token_lens.push(p.hist_token_len);
                    flat.is_prefill.push(true);
                    flat.is_retract_recovery.push(p.is_retract_recovery);
                    flat.paged_cache_pages.push(p.paged_cache_pages);
                    flat.paged_cache_page_base_offsets.push(p.paged_cache_page_base_offsets);
                }
                ForwardOperation::Decode(d) => {
                    flat.request_ids.push(d.request_id);
                    flat.input_ids.push(d.decode_input_id);
                    flat.shifted_input_ids.push(-1);
                    flat.occupied_pages.push(d.occupied_pages);
                    flat.page_sizes.push(d.page_size);
                    flat.extend_prefix_lens.push(0);
                    flat.begins.push(0);
                    flat.sizes.push(1);
                    flat.token_counts.push(d.token_count);
                    flat.decode_input_ids.push(d.decode_input_id);
                    flat.hist_token_lens.push(d.hist_token_len);
                    flat.is_prefill.push(false);
                    flat.is_retract_recovery.push(d.is_retract_recovery);
                    flat.paged_cache_pages.push(d.paged_cache_pages);
                    flat.paged_cache_page_base_offsets.push(d.paged_cache_page_base_offsets);
                }
            }
        }
        flat
    }

    pub fn is_empty(&self) -> bool {
        self.request_ids.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Cache operations
// ---------------------------------------------------------------------------

/// Cache operation type.
#[derive(Debug, Clone)]
pub enum CacheKind {
    KV,
    Mamba,
}

/// A cache operation for device↔host transfers.
#[derive(Debug, Clone)]
pub struct WriteBackOperation {
    pub request_id: String,
    pub device_pages: Vec<i32>,
    pub host_pages: Vec<i32>,
    pub is_retract: bool,
    pub kind: CacheKind,
}

/// A load-back operation for host→device recovery.
#[derive(Debug, Clone)]
pub struct LoadBackOperation {
    pub request_id: String,
    pub device_pages: Vec<i32>,
    pub host_pages: Vec<i32>,
    pub kind: CacheKind,
}

/// A prefetch operation for L3 storage.
#[derive(Debug, Clone)]
pub struct PrefetchOperation {
    pub request_id: String,
    pub host_pages: Vec<i32>,
    pub rolling_hashes: Vec<String>,
}
