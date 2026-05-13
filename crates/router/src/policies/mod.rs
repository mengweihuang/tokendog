//! Load-balancing policy traits and implementations.

pub mod least_loaded;
pub mod load_cache_aware;
pub mod power_of_two;
pub mod prefix_affinity;
pub mod random;
pub mod round_robin;
pub mod session_affinity;

/// Request fields extracted by the proxy handler for cache-aware routing.
///
/// Lightweight context parsed from the JSON request body so policies can
/// make decisions based on user identity and prompt content.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Identifier for the user/session — used by `SessionAffinity` and
    /// `LoadCacheAware` to route multi-turn conversations to the same worker.
    pub session_id: String,
    /// First 200 characters of the first message content (preferring
    /// `system`-role messages) — used by `PrefixAffinity` and
    /// `LoadCacheAware` to co-locate requests with shared system prompts.
    pub prefix_key: String,
}

/// Deterministic hash of a string, stable across process restarts.
///
/// Uses FNV-1a 64-bit so the same input always maps to the same worker,
/// even after the router restarts.
pub fn stable_hash(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Interface for worker selection policies.
///
/// Implementations must be [`Send`] + [`Sync`] so they can be shared across threads
/// in the axum application state.
pub trait LoadBalancer: Send + Sync {
    /// Select a worker index from the available worker URLs.
    ///
    /// # Panics
    ///
    /// Implementations may assume `workers` is non-empty but should not panic
    /// if it is empty (callers are responsible for validation).
    fn select(&self, workers: &[String]) -> usize;

    /// Select a worker index using request context for cache-aware routing.
    ///
    /// Default implementation delegates to [`select`](Self::select), so
    /// load-only policies (round-robin, random, least-loaded, etc.) work
    /// without changes.
    fn select_with_context(&self, workers: &[String], _ctx: &RequestContext) -> usize {
        self.select(workers)
    }

    /// Notify that a request has been dispatched to the given worker.
    ///
    /// Default is a no-op; policies that track in-flight requests should
    /// increment their counter for `worker_idx`.
    fn on_request_start(&self, _worker_idx: usize) {}

    /// Notify that a request to the given worker has completed (success or error).
    ///
    /// Default is a no-op; policies that track in-flight requests should
    /// decrement their counter for `worker_idx`.
    fn on_request_end(&self, _worker_idx: usize) {}

    /// Record a completed routing decision for future cache-affinity lookups.
    ///
    /// Called after the worker has responded. Default is a no-op;
    /// [`load_cache_aware::LoadCacheAware`] implements this to update its
    /// internal [`CacheDirectory`].
    fn record(&self, _ctx: &RequestContext, _worker_idx: usize) {}
}
