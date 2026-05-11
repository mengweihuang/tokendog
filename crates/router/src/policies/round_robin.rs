//! Round-robin worker selection policy.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::LoadBalancer;

/// A lock-free round-robin load balancer.
///
/// Distributes requests across workers by cycling through them sequentially.
/// Uses `Ordering::Relaxed` for the atomic counter — occasional stale reads
/// are acceptable since the target index is bounded by `workers.len()`.
pub struct RoundRobin {
    counter: AtomicUsize,
}

impl RoundRobin {
    /// Create a new `RoundRobin` balancer starting from the first worker.
    pub fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }
}

impl Default for RoundRobin {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadBalancer for RoundRobin {
    fn select(&self, workers: &[String]) -> usize {
        self.counter.fetch_add(1, Ordering::Relaxed) % workers.len()
    }
}
