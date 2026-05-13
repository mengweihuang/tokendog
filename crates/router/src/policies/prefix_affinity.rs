use std::sync::atomic::{AtomicUsize, Ordering};

use rand::Rng;

use crate::policies::{stable_hash, LoadBalancer, RequestContext};

/// Routes by first-message prefix hash, falling back to join-shortest-queue
/// when the preferred worker is overloaded.
///
/// # How it works
///
/// 1. Hash `prefix_key` → preferred worker index.
/// 2. If that worker has fewer than `threshold` in-flight requests, use it
///    (preserves cache affinity).
/// 3. Otherwise, pick the worker with the fewest in-flight requests
///    (random tie-breaking among the minimum).
///
/// The `threshold` parameter (default 10) acts as a circuit-breaker so a
/// single worker isnʼt swamped when many popular prefixes hash to it.
pub struct PrefixAffinity {
    active: Vec<AtomicUsize>,
    threshold: usize,
}

impl PrefixAffinity {
    /// Create a new `PrefixAffinity` policy.
    ///
    /// # Panics
    ///
    /// Panics if `worker_count` is 0.
    pub fn new(worker_count: usize) -> Self {
        Self::with_threshold(worker_count, 10)
    }

    /// Create a new `PrefixAffinity` policy with a custom threshold.
    pub fn with_threshold(worker_count: usize, threshold: usize) -> Self {
        assert!(worker_count > 0, "worker_count must be positive");
        let mut active = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            active.push(AtomicUsize::new(0));
        }
        Self { active, threshold }
    }
}

impl LoadBalancer for PrefixAffinity {
    fn select(&self, workers: &[String]) -> usize {
        if workers.len() == 1 {
            return 0;
        }
        // Without context, behave like Join-Shortest-Queue.
        jsq_fallback(&self.active, workers.len())
    }

    fn select_with_context(&self, workers: &[String], ctx: &RequestContext) -> usize {
        let n = workers.len();
        if n == 1 {
            return 0;
        }
        let preferred = (stable_hash(&ctx.prefix_key) as usize) % n;
        if self.active[preferred].load(Ordering::Relaxed) < self.threshold {
            return preferred;
        }
        jsq_fallback(&self.active, n)
    }

    fn on_request_start(&self, worker_idx: usize) {
        self.active[worker_idx].fetch_add(1, Ordering::Relaxed);
    }

    fn on_request_end(&self, worker_idx: usize) {
        self.active[worker_idx].fetch_sub(1, Ordering::Relaxed);
    }
}

/// Join-Shortest-Queue: pick the worker with the fewest active requests,
/// randomly breaking ties.
fn jsq_fallback(active: &[AtomicUsize], n: usize) -> usize {
    let mut rng = rand::thread_rng();
    let min_q = active
        .iter()
        .map(|a| a.load(Ordering::Relaxed))
        .min()
        .unwrap_or(0);
    let tied: Vec<usize> = (0..n)
        .filter(|i| active[*i].load(Ordering::Relaxed) == min_q)
        .collect();
    tied[rng.gen_range(0..tied.len())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_prefix_same_worker_under_threshold() {
        let policy = PrefixAffinity::new(5);
        let workers: Vec<String> = (0..5).map(|i| format!("http://w{}", i)).collect();
        let ctx = RequestContext {
            session_id: "s1".into(),
            prefix_key: "You are a helpful assistant...".into(),
        };

        let first = policy.select_with_context(&workers, &ctx);
        for _ in 0..10 {
            assert_eq!(policy.select_with_context(&workers, &ctx), first);
        }
    }

    #[test]
    fn falls_back_when_overloaded() {
        let policy = PrefixAffinity::with_threshold(3, 2);
        let workers: Vec<String> = (0..3).map(|i| format!("http://w{}", i)).collect();
        let ctx = RequestContext {
            session_id: "s1".into(),
            prefix_key: "system-prompt-a".into(),
        };

        // Determine which worker is the preferred one.
        let preferred = policy.select_with_context(&workers, &ctx);

        // Saturate the preferred worker.
        policy.on_request_start(preferred);
        policy.on_request_start(preferred);

        // Now preferred is at threshold (2), should fall back.
        let fallback = policy.select_with_context(&workers, &ctx);
        assert_ne!(fallback, preferred);

        // Clean up.
        policy.on_request_end(preferred);
        policy.on_request_end(preferred);
    }
}
