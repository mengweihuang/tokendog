//! Load-balancing policy traits and implementations.

pub mod round_robin;

/// Interface for backend selection policies.
///
/// Implementations must be [`Send`] + [`Sync`] so they can be shared across threads
/// in the axum application state.
pub trait LoadBalancer: Send + Sync {
    /// Select a backend index from the available backends.
    ///
    /// # Panics
    ///
    /// Implementations may assume `backends` is non-empty but should not panic
    /// if it is empty (callers are responsible for validation).
    fn select(&self, backends: &[String]) -> usize;
}
