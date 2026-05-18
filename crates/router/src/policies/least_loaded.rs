//! Least-loaded worker selection policy with active-request tracking.

use std::sync::atomic::{AtomicUsize, Ordering};
use rand::Rng;
use super::LoadBalancer;

/// A least-loaded load balancer.
///
/// Maintains per-worker active-request counters. On each call scans all workers
/// and selects the one with the fewest in-flight requests.
pub struct LeastLoaded {
    active: Vec<AtomicUsize>,
}

impl LeastLoaded {
    /// Create a new balancer for `worker_count` workers.
    pub fn new(worker_count: usize) -> Self {
        Self {
            active: (0..worker_count).map(|_| AtomicUsize::new(0)).collect(),
        }
    }
}

impl LoadBalancer for LeastLoaded {
    fn select(&self, workers: &[String]) -> usize {
        let n = workers.len();
        let mut best = rand::thread_rng().gen_range(0..n);
        let mut least = self.active[0].load(Ordering::Relaxed);
        for i in 1..n {
            let load = self.active[i].load(Ordering::Relaxed);
            if load < least {
                best = i;
                least = load;
            }
        }
        best
    }

    fn on_request_start(&self, worker_idx: usize) {
        self.active[worker_idx].fetch_add(1, Ordering::Relaxed);
    }

    fn on_request_end(&self, worker_idx: usize) {
        self.active[worker_idx].fetch_sub(1, Ordering::Relaxed);
    }
}
