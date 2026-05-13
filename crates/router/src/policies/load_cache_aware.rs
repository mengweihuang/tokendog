use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::Rng;

use crate::policies::{LoadBalancer, RequestContext};

/// Tracks which worker most recently served each prefix and session so
/// `LoadCacheAware` can compute cache-affinity scores.
///
/// Maintains two maps: `prefix_map` (keyed by first-message content hash)
/// and `session_map` (keyed by user/session id). Each entry records the
/// worker index and insertion time for staleness checks.
struct CacheDirectory {
    prefix_map: HashMap<String, (usize, Instant)>,
    session_map: HashMap<String, (usize, Instant)>,
    staleness: Duration,
}

impl CacheDirectory {
    fn new(staleness_secs: f64) -> Self {
        Self {
            prefix_map: HashMap::new(),
            session_map: HashMap::new(),
            staleness: Duration::from_secs_f64(staleness_secs),
        }
    }

    /// Return a per-worker affinity vector (length = `n_workers`).
    ///
    /// - `1.0` if this worker has a non-stale prefix-key entry.
    /// - `0.5` if this worker has a non-stale session-id entry (overrides
    ///   a stale prefix match).
    /// - `0.0` otherwise.
    fn get_affinity(&self, prefix_key: &str, session_id: &str, n_workers: usize) -> Vec<f64> {
        let now = Instant::now();
        let mut scores = vec![0.0f64; n_workers];

        if self.staleness == Duration::ZERO {
            // Fast path: staleness disabled.
            if let Some(&(idx, _)) = self.prefix_map.get(prefix_key) {
                if idx < n_workers {
                    scores[idx] = 1.0;
                }
            }
            if let Some(&(idx, _)) = self.session_map.get(session_id) {
                if idx < n_workers {
                    scores[idx] = 0.5;
                }
            }
        } else {
            if let Some(&(idx, ts)) = self.prefix_map.get(prefix_key) {
                if idx < n_workers && now.duration_since(ts) < self.staleness {
                    scores[idx] = 1.0;
                }
            }
            if let Some(&(idx, ts)) = self.session_map.get(session_id) {
                if idx < n_workers && now.duration_since(ts) < self.staleness {
                    scores[idx] = 0.5;
                }
            }
        }
        scores
    }

    fn record(&mut self, prefix_key: &str, session_id: &str, worker_idx: usize) {
        let now = Instant::now();
        self.prefix_map
            .insert(prefix_key.to_string(), (worker_idx, now));
        self.session_map
            .insert(session_id.to_string(), (worker_idx, now));
    }
}

/// Scores workers by `alpha * cache_affinity - beta * normalized_load`,
/// selecting the highest-scoring worker (random tie-breaking).
///
/// # Scoring formula
///
/// For each worker *i*:
///
/// ```text
/// score[i] = alpha * affinity[i] - beta * (active[i] / max(active, 1))
/// ```
///
/// - `affinity[i]` comes from the internal [`CacheDirectory`]:
///   1.0 (prefix cached), 0.5 (session cached), 0.0 (nothing).
/// - `active[i]` is the current in-flight request count.
/// - `alpha` (default 0.7) weights cache locality.
/// - `beta` (default 0.3) weights load balancing.
///
/// The `cache_staleness_secs` parameter (default 0 = never stale) ages
/// out old cache-directory entries so workers that have evicted cached
/// prefixes are no longer preferred.
pub struct LoadCacheAware {
    active: Vec<AtomicUsize>,
    cache_dir: Mutex<CacheDirectory>,
    alpha: f64,
    beta: f64,
}

impl LoadCacheAware {
    /// Create a new `LoadCacheAware` policy with default weights
    /// (`alpha = 0.7`, `beta = 0.3`, staleness disabled).
    ///
    /// # Panics
    ///
    /// Panics if `worker_count` is 0.
    pub fn new(worker_count: usize) -> Self {
        Self::with_params(worker_count, 0.7, 0.3, 0.0)
    }

    /// Create with custom scoring weights and cache staleness.
    ///
    /// `alpha` weights cache affinity; `beta` weights load pressure.
    /// `cache_staleness_secs` (0 = never consider entries stale).
    pub fn with_params(
        worker_count: usize,
        alpha: f64,
        beta: f64,
        cache_staleness_secs: f64,
    ) -> Self {
        assert!(worker_count > 0, "worker_count must be positive");
        let mut active = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            active.push(AtomicUsize::new(0));
        }
        Self {
            active,
            cache_dir: Mutex::new(CacheDirectory::new(cache_staleness_secs)),
            alpha,
            beta,
        }
    }
}

impl LoadBalancer for LoadCacheAware {
    fn select(&self, workers: &[String]) -> usize {
        if workers.len() == 1 {
            return 0;
        }
        // Without context, behave like Join-Shortest-Queue.
        let mut rng = rand::thread_rng();
        let min_q = self
            .active
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .min()
            .unwrap_or(0);
        let tied: Vec<usize> = (0..workers.len())
            .filter(|i| self.active[*i].load(Ordering::Relaxed) == min_q)
            .collect();
        tied[rng.gen_range(0..tied.len())]
    }

    fn select_with_context(&self, workers: &[String], ctx: &RequestContext) -> usize {
        let n = workers.len();
        if n == 1 {
            return 0;
        }

        let affinity = {
            let cache = self.cache_dir.lock().unwrap();
            cache.get_affinity(&ctx.prefix_key, &ctx.session_id, n)
        };

        let loads: Vec<usize> = self
            .active
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .collect();
        let max_load = loads.iter().max().copied().unwrap_or(1).max(1) as f64;

        let mut best_score = f64::NEG_INFINITY;
        let mut best_indices = Vec::new();

        for i in 0..n {
            let load_score = loads[i] as f64 / max_load;
            let score = self.alpha * affinity[i] - self.beta * load_score;
            match score.partial_cmp(&best_score) {
                Some(std::cmp::Ordering::Greater) => {
                    best_score = score;
                    best_indices.clear();
                    best_indices.push(i);
                }
                Some(std::cmp::Ordering::Equal) => {
                    best_indices.push(i);
                }
                _ => {} // NaN — skip
            }
        }

        let mut rng = rand::thread_rng();
        best_indices[rng.gen_range(0..best_indices.len())]
    }

    fn on_request_start(&self, worker_idx: usize) {
        self.active[worker_idx].fetch_add(1, Ordering::Relaxed);
    }

    fn on_request_end(&self, worker_idx: usize) {
        self.active[worker_idx].fetch_sub(1, Ordering::Relaxed);
    }

    fn record(&self, ctx: &RequestContext, worker_idx: usize) {
        let mut cache = self.cache_dir.lock().unwrap();
        cache.record(&ctx.prefix_key, &ctx.session_id, worker_idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affinity_grows_after_record() {
        let policy = LoadCacheAware::with_params(3, 1.0, 0.0, 0.0); // alpha=1, beta=0: pure cache
        let workers: Vec<String> = (0..3).map(|i| format!("http://w{}", i)).collect();
        let ctx = RequestContext {
            session_id: "u1".into(),
            prefix_key: "sys-a".into(),
        };

        // First request: no affinity → all scores 0 → random tie-break.
        let first = policy.select_with_context(&workers, &ctx);

        // Record the decision.
        policy.record(&ctx, first);

        // Second request: should now prefer the recorded worker.
        let second = policy.select_with_context(&workers, &ctx);
        assert_eq!(second, first);
    }

    #[test]
    fn load_avoids_overloaded_worker() {
        let policy = LoadCacheAware::with_params(3, 1.0, 10.0, 0.0);
        let workers: Vec<String> = (0..3).map(|i| format!("http://w{}", i)).collect();
        let ctx = RequestContext {
            session_id: "u1".into(),
            prefix_key: "sys-a".into(),
        };

        // Record worker 0 as having the cache.
        policy.record(&ctx, 0);

        // Saturate worker 0.
        for _ in 0..100 {
            policy.on_request_start(0);
        }

        // Should avoid worker 0 despite high affinity.
        let chosen = policy.select_with_context(&workers, &ctx);
        assert_ne!(chosen, 0);

        // Clean up.
        for _ in 0..100 {
            policy.on_request_end(0);
        }
    }
}
