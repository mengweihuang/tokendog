//! Application state with load-balancing policy and shared HTTP client.

use std::time::Duration;

use crate::policies::LoadBalancer;

/// Shared application state holding worker URL list, HTTP client, and load balancer.
pub struct AppState {
    /// List of worker URLs.
    pub worker_urls: Vec<String>,
    /// Reusable HTTP client with connection pooling and timeout.
    pub client: reqwest::Client,
    /// Worker selection policy.
    balancer: Box<dyn LoadBalancer>,
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
        balancer: impl LoadBalancer + 'static,
    ) -> Self {
        assert!(
            !worker_urls.is_empty(),
            "AppState requires at least one worker URL"
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("Failed to build reqwest Client");

        Self {
            worker_urls,
            client,
            balancer: Box::new(balancer),
        }
    }

    /// Return the index and URL of the next worker using the configured policy.
    ///
    /// Also notifies the balancer that a request has started on this worker.
    pub fn next_worker(&self) -> (usize, &str) {
        let idx = self.balancer.select(&self.worker_urls);
        self.balancer.on_request_start(idx);
        (idx, &self.worker_urls[idx])
    }

    /// Notify the balancer that the request to `worker_idx` has completed.
    pub fn finish_request(&self, worker_idx: usize) {
        self.balancer.on_request_end(worker_idx);
    }
}
