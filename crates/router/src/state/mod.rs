//! Application state with load-balancing policy and shared HTTP client.

use std::time::Duration;

use crate::config;
use crate::policies::{LoadBalancer, RequestContext};

/// Shared application state holding worker URL list, HTTP client, and load balancer.
pub struct AppState {
    /// List of worker URLs.
    pub worker_urls: Vec<String>,
    /// Reusable HTTP client with connection pooling and timeout.
    pub client: reqwest::Client,
    /// Worker selection policy.
    balancer: Box<dyn LoadBalancer>,

    // ── PD separation fields (only populated in PD mode) ──
    /// Which PD mode is active, if any.
    pd_mode: Option<config::PdMode>,
    /// Prefill-dedicated worker URLs.
    prefill_urls: Vec<String>,
    /// Decode-dedicated worker URLs.
    decode_urls: Vec<String>,
    /// Worker selection policy for the prefill pool.
    prefill_policy: Option<Box<dyn LoadBalancer>>,
    /// Worker selection policy for the decode pool.
    decode_policy: Option<Box<dyn LoadBalancer>>,
}

impl AppState {
    /// Create a new `AppState` with the given worker URLs, request timeout, and policy.
    ///
    /// # Panics
    ///
    /// Panics if `worker_urls` is empty.
    pub fn new(
        worker_urls: Vec<String>,
        timeout_secs: u64,
        balancer: Box<dyn LoadBalancer>,
    ) -> Self {
        assert!(
            !worker_urls.is_empty(),
            "AppState requires at least one worker URL"
        );

        Self {
            worker_urls,
            client: build_client(timeout_secs),
            balancer,
            pd_mode: None,
            prefill_urls: Vec::new(),
            decode_urls: Vec::new(),
            prefill_policy: None,
            decode_policy: None,
        }
    }

    /// Create PD-mode `AppState` with separate prefill and decode worker pools.
    ///
    /// The fallback `worker_urls` / `balancer` are set to the decode pool so
    /// non-inference requests forwarded by the regular proxy handler target
    /// decode workers.
    ///
    /// # Panics
    ///
    /// Panics if either `prefill_urls` or `decode_urls` is empty.
    pub fn new_pd(
        pd_mode: config::PdMode,
        prefill_urls: Vec<String>,
        decode_urls: Vec<String>,
        timeout_secs: u64,
        prefill_policy: Box<dyn LoadBalancer>,
        decode_policy: Box<dyn LoadBalancer>,
    ) -> Self {
        assert!(
            !prefill_urls.is_empty(),
            "AppState::new_pd requires at least one prefill URL"
        );
        assert!(
            !decode_urls.is_empty(),
            "AppState::new_pd requires at least one decode URL"
        );

        // Fallback balancer for non-inference paths targets decode workers.
        // Construct a lightweight round-robin for the fallback to avoid
        // double-counting on the decode pool.
        use crate::policies::round_robin::RoundRobin;
        let fallback: Box<dyn LoadBalancer> = Box::new(RoundRobin::new());

        Self {
            worker_urls: decode_urls.clone(),
            client: build_client(timeout_secs),
            balancer: fallback,
            pd_mode: Some(pd_mode),
            prefill_urls,
            decode_urls,
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

    /// Select a prefill worker and notify the policy that a request started.
    pub fn next_prefill_worker(&self, ctx: &RequestContext) -> Option<(usize, &str)> {
        let policy = self.prefill_policy.as_ref()?;
        let idx = policy.select_with_context(&self.prefill_urls, ctx);
        policy.on_request_start(idx);
        Some((idx, &self.prefill_urls[idx]))
    }

    /// Select a decode worker and notify the policy that a request started.
    pub fn next_decode_worker(&self, ctx: &RequestContext) -> Option<(usize, &str)> {
        let policy = self.decode_policy.as_ref()?;
        let idx = policy.select_with_context(&self.decode_urls, ctx);
        policy.on_request_start(idx);
        Some((idx, &self.decode_urls[idx]))
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

    /// Return the index and URL of the next worker using the configured policy.
    ///
    /// Also notifies the balancer that a request has started on this worker.
    /// Prefer [`next_worker_with_context`](Self::next_worker_with_context) when
    /// request context is available so cache-aware policies can use it.
    pub fn next_worker(&self) -> (usize, &str) {
        let idx = self.balancer.select(&self.worker_urls);
        self.balancer.on_request_start(idx);
        (idx, &self.worker_urls[idx])
    }

    /// Return the index and URL of the next worker, using request context for
    /// cache-aware routing decisions.
    pub fn next_worker_with_context(&self, ctx: &RequestContext) -> (usize, &str) {
        let idx = self.balancer.select_with_context(&self.worker_urls, ctx);
        self.balancer.on_request_start(idx);
        (idx, &self.worker_urls[idx])
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

fn build_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .expect("Failed to build reqwest Client")
}
