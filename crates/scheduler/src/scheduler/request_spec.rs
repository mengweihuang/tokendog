//! Request specification submitted to the scheduler.

/// Input specification for a new request.
#[derive(Debug, Clone)]
pub struct RequestSpec {
    /// Unique request identifier.
    pub request_id: String,
    /// Token IDs for the request (prompt tokens).
    pub tokens: Vec<i32>,
    /// Rolling SHA-256 hashes for L3 storage lookup.
    pub rolling_hashes: Vec<String>,
    /// Number of storage-hit pages (L3 cache hit).
    pub storage_hit_pages: i32,
}

/// Information about a prefill window.
#[derive(Debug, Clone)]
pub struct PrefillInfo {
    pub input_ids: Vec<i32>,
    pub shifted_input_ids: Vec<i32>,
    pub already_scheduled_len: i32,
    pub extend_len: i32,
}

/// Storage info for a request.
#[derive(Debug, Clone, Default)]
pub struct StorageInfo {
    pub hit_pages: i32,
}
