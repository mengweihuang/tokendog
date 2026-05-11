//! Load-balancing policy traits and implementations.

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
}
