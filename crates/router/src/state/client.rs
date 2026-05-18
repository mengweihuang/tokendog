//! Reqwest HTTP client construction.

use std::time::Duration;

/// Build a reusable HTTP client with connection pooling and timeout.
pub(super) fn build_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .expect("Failed to build reqwest Client")
}
