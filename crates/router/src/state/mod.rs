//! Application state with load-balancing policy and shared HTTP client.

pub mod client;

use std::sync::Arc;

use crate::config;
use crate::policies::{LoadBalancer, RequestContext};
use crate::worker::{self, Worker};

use self::client::build_client;

/// Shared application state holding worker list, HTTP client, and load balancer.
pub struct AppState {
    /// Worker pool (regular mode).
    pub workers: Vec<Arc<Worker>>,
    /// Reusable HTTP client with connection pooling and timeout.
    pub client: reqwest::Client,
    /// Worker selection policy.
    balancer: Box<dyn LoadBalancer>,

    // ── PD separation fields (only populated in PD mode) ──
    /// Which PD mode is active, if any.
    pd_mode: Option<config::PdMode>,
    /// Prefill-dedicated workers.
    prefill_workers: Vec<Arc<Worker>>,
    /// Decode-dedicated workers.
    decode_workers: Vec<Arc<Worker>>,
    /// Worker selection policy for the prefill pool.
    prefill_policy: Option<Box<dyn LoadBalancer>>,
    /// Worker selection policy for the decode pool.
    decode_policy: Option<Box<dyn LoadBalancer>>,
}

impl AppState {
    /// Create a new `AppState` with the given workers, request timeout, and policy.
    ///
    /// # Panics
    ///
    /// Panics if `workers` is empty.
    pub fn new(
        workers: Vec<Arc<Worker>>,
        timeout_secs: u64,
        balancer: Box<dyn LoadBalancer>,
    ) -> Self {
        assert!(
            !workers.is_empty(),
            "AppState requires at least one worker URL"
        );

        Self {
            workers,
            client: build_client(timeout_secs),
            balancer,
            pd_mode: None,
            prefill_workers: Vec::new(),
            decode_workers: Vec::new(),
            prefill_policy: None,
            decode_policy: None,
        }
    }

    /// Create PD-mode `AppState` with separate prefill and decode worker pools.
    ///
    /// The fallback `workers` / `balancer` are set to the decode pool so
    /// non-inference requests forwarded by the regular proxy handler target
    /// decode workers.
    ///
    /// # Panics
    ///
    /// Panics if either `prefill_workers` or `decode_workers` is empty.
    pub fn new_pd(
        pd_mode: config::PdMode,
        prefill_workers: Vec<Arc<Worker>>,
        decode_workers: Vec<Arc<Worker>>,
        timeout_secs: u64,
        prefill_policy: Box<dyn LoadBalancer>,
        decode_policy: Box<dyn LoadBalancer>,
    ) -> Self {
        assert!(
            !prefill_workers.is_empty(),
            "AppState::new_pd requires at least one prefill URL"
        );
        assert!(
            !decode_workers.is_empty(),
            "AppState::new_pd requires at least one decode URL"
        );

        // Fallback balancer for non-inference paths targets decode workers.
        // Construct a lightweight round-robin for the fallback to avoid
        // double-counting on the decode pool.
        use crate::policies::round_robin::RoundRobin;
        let fallback: Box<dyn LoadBalancer> = Box::new(RoundRobin::new());

        Self {
            workers: decode_workers.clone(),
            client: build_client(timeout_secs),
            balancer: fallback,
            pd_mode: Some(pd_mode),
            prefill_workers,
            decode_workers,
            prefill_policy: Some(prefill_policy),
            decode_policy: Some(decode_policy),
        }
    }

    /// Whether PD mode is active.
    pub fn is_pd_mode(&self) -> bool {
        self.pd_mode.is_some()
    }

    /// Return the active PD mode, if any.
    pub fn pd_mode(&self) -> Option<config::PdMode> {
        self.pd_mode
    }

    /// Return a reference to the prefill worker list (for health checks).
    pub fn prefill_workers(&self) -> &[Arc<Worker>] {
        &self.prefill_workers
    }

    /// Return a reference to the decode worker list (for health checks).
    pub fn decode_workers(&self) -> &[Arc<Worker>] {
        &self.decode_workers
    }

    /// Select a healthy prefill worker and notify the policy that a request started.
    ///
    /// Returns `None` if no healthy prefill workers are available.
    pub fn next_prefill_worker(&self, ctx: &RequestContext) -> Option<(usize, &str)> {
        let policy = self.prefill_policy.as_ref()?;
        let healthy = worker::healthy_worker_urls(&self.prefill_workers);
        if healthy.is_empty() {
            return None;
        }
        let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
        let filtered_idx = policy.select_with_context(&urls, ctx);
        let (original_idx, worker_url) = healthy[filtered_idx];
        policy.on_request_start(original_idx);
        Some((original_idx, worker_url))
    }

    /// Select a healthy decode worker and notify the policy that a request started.
    ///
    /// Returns `None` if no healthy decode workers are available.
    pub fn next_decode_worker(&self, ctx: &RequestContext) -> Option<(usize, &str)> {
        let policy = self.decode_policy.as_ref()?;
        let healthy = worker::healthy_worker_urls(&self.decode_workers);
        if healthy.is_empty() {
            return None;
        }
        let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
        let filtered_idx = policy.select_with_context(&urls, ctx);
        let (original_idx, worker_url) = healthy[filtered_idx];
        policy.on_request_start(original_idx);
        Some((original_idx, worker_url))
    }

    /// Notify the prefill policy that the request to `worker_idx` completed.
    pub fn finish_prefill_request(&self, worker_idx: usize) {
        if let Some(policy) = self.prefill_policy.as_ref() {
            policy.on_request_end(worker_idx);
        }
    }

    /// Notify the decode policy that the request to `worker_idx` completed.
    pub fn finish_decode_request(&self, worker_idx: usize) {
        if let Some(policy) = self.decode_policy.as_ref() {
            policy.on_request_end(worker_idx);
        }
    }

    /// Record a completed prefill routing decision for cache-aware policies.
    pub fn record_prefill_request(&self, ctx: &RequestContext, worker_idx: usize) {
        if let Some(policy) = self.prefill_policy.as_ref() {
            policy.record(ctx, worker_idx);
        }
    }

    /// Record a completed decode routing decision for cache-aware policies.
    pub fn record_decode_request(&self, ctx: &RequestContext, worker_idx: usize) {
        if let Some(policy) = self.decode_policy.as_ref() {
            policy.record(ctx, worker_idx);
        }
    }

    /// Return the index and URL of the next healthy worker using the configured policy.
    ///
    /// Also notifies the balancer that a request has started on this worker.
    ///
    /// Returns `None` if no healthy workers are available.
    pub fn next_worker(&self) -> Option<(usize, &str)> {
        let healthy = worker::healthy_worker_urls(&self.workers);
        if healthy.is_empty() {
            return None;
        }
        let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
        let filtered_idx = self.balancer.select(&urls);
        let (original_idx, worker_url) = healthy[filtered_idx];
        self.balancer.on_request_start(original_idx);
        Some((original_idx, worker_url))
    }

    /// Return the index and URL of the next healthy worker, using request context
    /// for cache-aware routing decisions.
    ///
    /// Returns `None` if no healthy workers are available.
    pub fn next_worker_with_context(&self, ctx: &RequestContext) -> Option<(usize, &str)> {
        let healthy = worker::healthy_worker_urls(&self.workers);
        if healthy.is_empty() {
            return None;
        }
        let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
        let filtered_idx = self.balancer.select_with_context(&urls, ctx);
        let (original_idx, worker_url) = healthy[filtered_idx];
        self.balancer.on_request_start(original_idx);
        Some((original_idx, worker_url))
    }

    /// Notify the balancer that the request to `worker_idx` has completed.
    pub fn finish_request(&self, worker_idx: usize) {
        self.balancer.on_request_end(worker_idx);
    }

    /// Record a completed routing decision so cache-aware policies can update
    /// their affinity state for future requests.
    pub fn record_request(&self, ctx: &RequestContext, worker_idx: usize) {
        self.balancer.record(ctx, worker_idx);
    }
}
