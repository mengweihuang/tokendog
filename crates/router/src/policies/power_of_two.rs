//! Power-of-two-choices worker selection policy with active-request tracking.

use std::sync::atomic::{AtomicUsize, Ordering};

use rand::Rng;

use super::LoadBalancer;

/// A power-of-two-choices load balancer with true least-loaded selection.
///
/// Maintains per-worker active-request counters. On each call picks two workers
/// uniformly at random and selects the one with fewer in-flight requests.
pub struct PowerOfTwo {
    active: Vec<AtomicUsize>,
}

impl PowerOfTwo {
    /// Create a new balancer for `worker_count` workers.
    pub fn new(worker_count: usize) -> Self {
        Self {
            active: (0..worker_count).map(|_| AtomicUsize::new(0)).collect(),
        }
    }
}

impl LoadBalancer for PowerOfTwo {
    fn select(&self, workers: &[String]) -> usize {
        let n = workers.len();
        let a = rand::thread_rng().gen_range(0..n);
        let b = rand::thread_rng().gen_range(0..n);
        if self.active[a].load(Ordering::Relaxed) <= self.active[b].load(Ordering::Relaxed) {
            a
        } else {
            b
        }
    }

    fn on_request_start(&self, worker_idx: usize) {
        self.active[worker_idx].fetch_add(1, Ordering::Relaxed);
    }

    fn on_request_end(&self, worker_idx: usize) {
        self.active[worker_idx].fetch_sub(1, Ordering::Relaxed);
    }
}
