//! Worker node abstraction with health status tracking.

pub mod health;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A worker node identified by URL with an atomic health flag.
///
/// The health flag is updated by the background health-check task and read
/// on the hot path during worker selection. Workers start optimistic
/// (healthy) so they can serve requests immediately, before the first
/// health check completes.
pub struct Worker {
    pub url: String,
    healthy: AtomicBool,
}

impl Worker {
    /// Create a new worker, initially marked healthy.
    pub fn new(url: String) -> Self {
        Self {
            url,
            healthy: AtomicBool::new(true),
        }
    }

    /// Create `Arc<Worker>` instances from a list of URL strings.
    pub fn from_urls(urls: &[String]) -> Vec<Arc<Self>> {
        urls.iter()
            .map(|url| Arc::new(Self::new(url.clone())))
            .collect()
    }

    /// Returns `true` if this worker is currently healthy.
    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    /// Update the health status of this worker.
    pub fn set_healthy(&self, healthy: bool) {
        self.healthy.store(healthy, Ordering::Relaxed);
    }
}

/// Build a filtered list of `(original_index, &url)` for healthy workers
/// from a full worker pool.
///
/// The original index maps back to the full `Vec<Arc<Worker>>` so that
/// policy callbacks (`on_request_start`, `on_request_end`) receive the
/// correct index into their pre-allocated `active` vectors.
pub fn healthy_worker_urls(workers: &[Arc<Worker>]) -> Vec<(usize, &str)> {
    workers
        .iter()
        .enumerate()
        .filter(|(_, w)| w.is_healthy())
        .map(|(i, w)| (i, w.url.as_str()))
        .collect()
}

/// Probe a single worker's `/health` endpoint.
///
/// Returns `true` if the worker responds with HTTP 200.
pub async fn check_worker_health(client: &reqwest::Client, url: &str) -> bool {
    let health_url = format!("{}/health", url.trim_end_matches('/'));
    match client.get(&health_url).send().await {
        Ok(resp) => resp.status() == reqwest::StatusCode::OK,
        Err(_) => false,
    }
}

/// Run one round of health checks against a set of workers, updating each
/// worker's health flag. Logs every check result; the `changed` field
/// indicates whether the health status transitioned.
pub async fn run_health_checks(client: &reqwest::Client, workers: &[Arc<Worker>]) {
    for worker in workers {
        let was_healthy = worker.is_healthy();
        let healthy = check_worker_health(client, &worker.url).await;
        worker.set_healthy(healthy);
        tracing::info!(
            url = %worker.url,
            healthy = healthy,
            changed = healthy != was_healthy,
            "Health check",
        );
    }
}
