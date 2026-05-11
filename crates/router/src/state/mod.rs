//! Application state with load-balancing policy and shared HTTP client.

use std::time::Duration;

use crate::policies::LoadBalancer;

/// Shared application state holding backend list, HTTP client, and load balancer.
pub struct AppState {
    /// List of backend URLs.
    pub backends: Vec<String>,
    /// Reusable HTTP client with connection pooling and timeout.
    pub client: reqwest::Client,
    /// Backend selection policy.
    balancer: Box<dyn LoadBalancer>,
}

impl AppState {
    /// Create a new `AppState` with the given backends, request timeout, and policy.
    ///
    /// # Panics
    ///
    /// Panics if `backends` is empty.
    pub fn new(
        backends: Vec<String>,
        timeout_secs: u64,
        balancer: impl LoadBalancer + 'static,
    ) -> Self {
        assert!(
            !backends.is_empty(),
            "AppState requires at least one backend"
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("Failed to build reqwest Client");

        Self {
            backends,
            client,
            balancer: Box::new(balancer),
        }
    }

    /// Return the next backend URL using the configured load-balancing policy.
    pub fn next_backend(&self) -> &str {
        let idx = self.balancer.select(&self.backends);
        &self.backends[idx]
    }
}
