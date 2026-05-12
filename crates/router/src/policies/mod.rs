//! Load-balancing policy traits and implementations.

pub mod least_loaded;
pub mod power_of_two;
pub mod random;
pub mod round_robin;

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
}
