//! Server setup and application state.
//!
//! Contains AppState (worker pool, HTTP client, load balancer), the axum
//! router builder, and the graceful-shutdown signal handler.

use std::future;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::{middleware, routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::config;
use crate::config::auth::AuthConfig;
use crate::policies::{LoadBalancer, RequestContext};
use crate::service_discovery::SharedWorkerPool;
use crate::worker::{self, Worker};

// ── HTTP client ────────────────────────────────────────────────────────────

/// Build a reusable HTTP client with connection pooling and timeout.
fn build_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .expect("Failed to build reqwest Client")
}

// ── Application state ──────────────────────────────────────────────────────

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

    // ── K8s service discovery fields ──────────────────────────────────────
    /// Whether workers are managed via K8s service discovery.
    k8s_mode: bool,
    /// Dynamic worker pool managed by K8s service discovery (regular mode).
    k8s_workers_pool: Option<SharedWorkerPool>,
    /// Dynamic prefill pool (PD mode).
    k8s_prefill_pool: Option<SharedWorkerPool>,
    /// Dynamic decode pool (PD mode).
    k8s_decode_pool: Option<SharedWorkerPool>,
    /// Policy type for rebuild when worker count changes.
    policy_type: Option<config::Policy>,
    /// Rebuildable policy for K8s regular mode.
    k8s_balancer: Option<Arc<RwLock<Box<dyn LoadBalancer>>>>,
    /// Rebuildable prefill policy for K8s PD mode.
    k8s_prefill_balancer: Option<Arc<RwLock<Box<dyn LoadBalancer>>>>,
    /// Rebuildable decode policy for K8s PD mode.
    k8s_decode_balancer: Option<Arc<RwLock<Box<dyn LoadBalancer>>>>,
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
            k8s_mode: false,
            k8s_workers_pool: None,
            k8s_prefill_pool: None,
            k8s_decode_pool: None,
            policy_type: None,
            k8s_balancer: None,
            k8s_prefill_balancer: None,
            k8s_decode_balancer: None,
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
            k8s_mode: false,
            k8s_workers_pool: None,
            k8s_prefill_pool: None,
            k8s_decode_pool: None,
            policy_type: None,
            k8s_balancer: None,
            k8s_prefill_balancer: None,
            k8s_decode_balancer: None,
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

    // ── K8s constructors ──────────────────────────────────────────────────

    /// Create a K8s-mode `AppState` with a dynamic worker pool.
    ///
    /// Workers are discovered and managed by the service discovery module.
    /// The pool starts empty and is populated as pods are discovered.
    pub fn new_k8s(
        timeout_secs: u64,
        policy_type: config::Policy,
        workers_pool: SharedWorkerPool,
    ) -> Self {
        let n = workers_pool.read().unwrap_or_else(|e| e.into_inner()).len();
        // Pre-allocate policy with generous capacity for dynamic scaling.
        let capacity = n.max(256);
        Self {
            workers: Vec::new(), // Not used in K8s mode
            client: build_client(timeout_secs),
            balancer: Self::make_policy(policy_type, n.max(1)), // fallback
            pd_mode: None,
            prefill_workers: Vec::new(),
            decode_workers: Vec::new(),
            prefill_policy: None,
            decode_policy: None,
            k8s_mode: true,
            k8s_workers_pool: Some(workers_pool),
            k8s_prefill_pool: None,
            k8s_decode_pool: None,
            policy_type: Some(policy_type),
            k8s_balancer: Some(Arc::new(RwLock::new(Self::make_policy(policy_type, capacity)))),
            k8s_prefill_balancer: None,
            k8s_decode_balancer: None,
        }
    }

    /// Create a K8s PD-mode `AppState` with separate prefill and decode pools.
    pub fn new_k8s_pd(
        pd_mode: config::PdMode,
        timeout_secs: u64,
        policy_type: config::Policy,
        prefill_pool: SharedWorkerPool,
        decode_pool: SharedWorkerPool,
    ) -> Self {
        let n_prefill = prefill_pool
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        let n_decode = decode_pool
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len();

        use crate::policies::round_robin::RoundRobin;
        let fallback: Box<dyn LoadBalancer> = Box::new(RoundRobin::new());

        // Pre-allocate policies with generous capacity for dynamic scaling.
        let capacity_prefill = n_prefill.max(256);
        let capacity_decode = n_decode.max(256);

        Self {
            workers: Vec::new(), // Not used in K8s mode
            client: build_client(timeout_secs),
            balancer: fallback,
            pd_mode: Some(pd_mode),
            prefill_workers: Vec::new(),
            decode_workers: Vec::new(),
            prefill_policy: None, // Not used in K8s PD mode
            decode_policy: None, // Not used in K8s PD mode
            k8s_mode: true,
            k8s_workers_pool: None,
            k8s_prefill_pool: Some(prefill_pool),
            k8s_decode_pool: Some(decode_pool),
            policy_type: Some(policy_type),
            k8s_balancer: None,
            k8s_prefill_balancer: Some(Arc::new(RwLock::new(Self::make_policy(
                policy_type,
                capacity_prefill,
            )))),
            k8s_decode_balancer: Some(Arc::new(RwLock::new(Self::make_policy(
                policy_type,
                capacity_decode,
            )))),
        }
    }

    /// Whether workers are managed via K8s service discovery.
    pub fn is_k8s_mode(&self) -> bool {
        self.k8s_mode
    }

    /// Build a `LoadBalancer` for the given policy and worker count.
    fn make_policy(policy: config::Policy, n: usize) -> Box<dyn LoadBalancer> {
        use crate::policies::{
            least_loaded::LeastLoaded, load_cache_aware::LoadCacheAware,
            power_of_two::PowerOfTwo, prefix_affinity::PrefixAffinity, random::Random,
            round_robin::RoundRobin, session_affinity::SessionAffinity,
        };
        let effective_n = n.max(1);
        match policy {
            config::Policy::LeastLoaded => Box::new(LeastLoaded::new(effective_n)),
            config::Policy::PowerOfTwo => Box::new(PowerOfTwo::new(effective_n)),
            config::Policy::Random => Box::new(Random),
            config::Policy::RoundRobin => Box::new(RoundRobin::new()),
            config::Policy::SessionAffinity => Box::new(SessionAffinity),
            config::Policy::PrefixAffinity => Box::new(PrefixAffinity::new(effective_n)),
            config::Policy::LoadCacheAware => Box::new(LoadCacheAware::new(effective_n)),
        }
    }

    /// Rebuild the policy from the current K8s worker pool size.
    ///
    /// Called by service discovery when workers are added or removed.
    pub fn rebuild_policy(&self) {
        let policy_type = match self.policy_type {
            Some(pt) => pt,
            None => return,
        };

        // Rebuild regular balancer if in K8s regular mode.
        if let (Some(ref pool), Some(ref balancer_lock)) =
            (&self.k8s_workers_pool, &self.k8s_balancer)
        {
            let n = pool.read().unwrap_or_else(|e| e.into_inner()).len();
            if let Ok(mut balancer) = balancer_lock.write() {
                *balancer = Self::make_policy(policy_type, n.max(1));
                tracing::info!("Rebuilt regular policy with n={}", n);
            }
        }

        // Rebuild prefill balancer.
        if let (Some(ref pool), Some(ref balancer_lock)) =
            (&self.k8s_prefill_pool, &self.k8s_prefill_balancer)
        {
            let n = pool.read().unwrap_or_else(|e| e.into_inner()).len();
            if let Ok(mut balancer) = balancer_lock.write() {
                *balancer = Self::make_policy(policy_type, n.max(1));
                tracing::info!("Rebuilt prefill policy with n={}", n);
            }
        }

        // Rebuild decode balancer.
        if let (Some(ref pool), Some(ref balancer_lock)) =
            (&self.k8s_decode_pool, &self.k8s_decode_balancer)
        {
            let n = pool.read().unwrap_or_else(|e| e.into_inner()).len();
            if let Ok(mut balancer) = balancer_lock.write() {
                *balancer = Self::make_policy(policy_type, n.max(1));
                tracing::info!("Rebuilt decode policy with n={}", n);
            }
        }
    }

    /// Return a snapshot of the K8s worker URLs (for health checks).
    pub fn k8s_worker_urls(&self) -> Vec<String> {
        self.k8s_workers_pool
            .as_ref()
            .map(|pool| {
                pool.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .map(|w| w.url.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return a snapshot of the K8s prefill worker URLs.
    pub fn k8s_prefill_urls(&self) -> Vec<String> {
        self.k8s_prefill_pool
            .as_ref()
            .map(|pool| {
                pool.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .map(|w| w.url.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return a snapshot of the K8s decode worker URLs.
    pub fn k8s_decode_urls(&self) -> Vec<String> {
        self.k8s_decode_pool
            .as_ref()
            .map(|pool| {
                pool.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .map(|w| w.url.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return a list of `Arc<Worker>` from a K8s pool for health checking.
    pub fn k8s_workers_snapshot(&self) -> Vec<Arc<Worker>> {
        self.k8s_workers_pool
            .as_ref()
            .map(|pool| {
                pool.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            })
            .unwrap_or_default()
    }

    /// Return a list of `Arc<Worker>` from the K8s prefill pool for health checking.
    pub fn k8s_prefill_snapshot(&self) -> Vec<Arc<Worker>> {
        self.k8s_prefill_pool
            .as_ref()
            .map(|pool| {
                pool.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            })
            .unwrap_or_default()
    }

    /// Return a list of `Arc<Worker>` from the K8s decode pool for health checking.
    pub fn k8s_decode_snapshot(&self) -> Vec<Arc<Worker>> {
        self.k8s_decode_pool
            .as_ref()
            .map(|pool| {
                pool.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            })
            .unwrap_or_default()
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
    pub fn next_prefill_worker(&self, ctx: &RequestContext) -> Option<(usize, String)> {
        if self.k8s_mode {
            let pool = self.k8s_prefill_pool.as_ref()?;
            let workers = pool.read().ok()?;
            let healthy = worker::healthy_worker_urls(&workers);
            if healthy.is_empty() {
                return None;
            }
            let balancer_lock = self.k8s_prefill_balancer.as_ref()?;
            let policy = balancer_lock.read().ok()?;
            let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
            let filtered_idx = policy.select_with_context(&urls, ctx);
            let (original_idx, worker_url) = healthy[filtered_idx];
            policy.on_request_start(original_idx);
            Some((original_idx, worker_url.to_string()))
        } else {
            let policy = self.prefill_policy.as_ref()?;
            let healthy = worker::healthy_worker_urls(&self.prefill_workers);
            if healthy.is_empty() {
                return None;
            }
            let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
            let filtered_idx = policy.select_with_context(&urls, ctx);
            let (original_idx, worker_url) = healthy[filtered_idx];
            policy.on_request_start(original_idx);
            Some((original_idx, worker_url.to_string()))
        }
    }

    /// Select a healthy decode worker and notify the policy that a request started.
    ///
    /// Returns `None` if no healthy decode workers are available.
    pub fn next_decode_worker(&self, ctx: &RequestContext) -> Option<(usize, String)> {
        if self.k8s_mode {
            let pool = self.k8s_decode_pool.as_ref()?;
            let workers = pool.read().ok()?;
            let healthy = worker::healthy_worker_urls(&workers);
            if healthy.is_empty() {
                return None;
            }
            let balancer_lock = self.k8s_decode_balancer.as_ref()?;
            let policy = balancer_lock.read().ok()?;
            let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
            let filtered_idx = policy.select_with_context(&urls, ctx);
            let (original_idx, worker_url) = healthy[filtered_idx];
            policy.on_request_start(original_idx);
            Some((original_idx, worker_url.to_string()))
        } else {
            let policy = self.decode_policy.as_ref()?;
            let healthy = worker::healthy_worker_urls(&self.decode_workers);
            if healthy.is_empty() {
                return None;
            }
            let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
            let filtered_idx = policy.select_with_context(&urls, ctx);
            let (original_idx, worker_url) = healthy[filtered_idx];
            policy.on_request_start(original_idx);
            Some((original_idx, worker_url.to_string()))
        }
    }

    /// Notify the prefill policy that the request to `worker_idx` completed.
    pub fn finish_prefill_request(&self, worker_idx: usize) {
        if self.k8s_mode {
            if let Some(ref lock) = self.k8s_prefill_balancer {
                if let Ok(policy) = lock.read() {
                    policy.on_request_end(worker_idx);
                }
            }
        } else if let Some(policy) = self.prefill_policy.as_ref() {
            policy.on_request_end(worker_idx);
        }
    }

    /// Notify the decode policy that the request to `worker_idx` completed.
    pub fn finish_decode_request(&self, worker_idx: usize) {
        if self.k8s_mode {
            if let Some(ref lock) = self.k8s_decode_balancer {
                if let Ok(policy) = lock.read() {
                    policy.on_request_end(worker_idx);
                }
            }
        } else if let Some(policy) = self.decode_policy.as_ref() {
            policy.on_request_end(worker_idx);
        }
    }

    /// Record a completed prefill routing decision for cache-aware policies.
    pub fn record_prefill_request(&self, ctx: &RequestContext, worker_idx: usize) {
        if self.k8s_mode {
            if let Some(ref lock) = self.k8s_prefill_balancer {
                if let Ok(policy) = lock.read() {
                    policy.record(ctx, worker_idx);
                }
            }
        } else if let Some(policy) = self.prefill_policy.as_ref() {
            policy.record(ctx, worker_idx);
        }
    }

    /// Record a completed decode routing decision for cache-aware policies.
    pub fn record_decode_request(&self, ctx: &RequestContext, worker_idx: usize) {
        if self.k8s_mode {
            if let Some(ref lock) = self.k8s_decode_balancer {
                if let Ok(policy) = lock.read() {
                    policy.record(ctx, worker_idx);
                }
            }
        } else if let Some(policy) = self.decode_policy.as_ref() {
            policy.record(ctx, worker_idx);
        }
    }

    /// Return the index and URL of the next healthy worker using the configured policy.
    ///
    /// Also notifies the balancer that a request has started on this worker.
    ///
    /// Returns `None` if no healthy workers are available.
    pub fn next_worker(&self) -> Option<(usize, String)> {
        if self.k8s_mode {
            let pool = self.k8s_workers_pool.as_ref()?;
            let workers = pool.read().ok()?;
            let healthy = worker::healthy_worker_urls(&workers);
            if healthy.is_empty() {
                return None;
            }
            let balancer_lock = self.k8s_balancer.as_ref()?;
            let balancer = balancer_lock.read().ok()?;
            let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
            let filtered_idx = balancer.select(&urls);
            let (original_idx, worker_url) = healthy[filtered_idx];
            balancer.on_request_start(original_idx);
            Some((original_idx, worker_url.to_string()))
        } else {
            let healthy = worker::healthy_worker_urls(&self.workers);
            if healthy.is_empty() {
                return None;
            }
            let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
            let filtered_idx = self.balancer.select(&urls);
            let (original_idx, worker_url) = healthy[filtered_idx];
            self.balancer.on_request_start(original_idx);
            Some((original_idx, worker_url.to_string()))
        }
    }

    /// Return the index and URL of the next healthy worker, using request context
    /// for cache-aware routing decisions.
    ///
    /// Returns `None` if no healthy workers are available.
    pub fn next_worker_with_context(&self, ctx: &RequestContext) -> Option<(usize, String)> {
        if self.k8s_mode {
            let pool = self.k8s_workers_pool.as_ref()?;
            let workers = pool.read().ok()?;
            let healthy = worker::healthy_worker_urls(&workers);
            if healthy.is_empty() {
                return None;
            }
            let balancer_lock = self.k8s_balancer.as_ref()?;
            let balancer = balancer_lock.read().ok()?;
            let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
            let filtered_idx = balancer.select_with_context(&urls, ctx);
            let (original_idx, worker_url) = healthy[filtered_idx];
            balancer.on_request_start(original_idx);
            Some((original_idx, worker_url.to_string()))
        } else {
            let healthy = worker::healthy_worker_urls(&self.workers);
            if healthy.is_empty() {
                return None;
            }
            let urls: Vec<String> = healthy.iter().map(|(_, url)| url.to_string()).collect();
            let filtered_idx = self.balancer.select_with_context(&urls, ctx);
            let (original_idx, worker_url) = healthy[filtered_idx];
            self.balancer.on_request_start(original_idx);
            Some((original_idx, worker_url.to_string()))
        }
    }

    /// Notify the balancer that the request to `worker_idx` has completed.
    pub fn finish_request(&self, worker_idx: usize) {
        if self.k8s_mode {
            if let Some(ref lock) = self.k8s_balancer {
                if let Ok(policy) = lock.read() {
                    policy.on_request_end(worker_idx);
                }
            }
        } else {
            self.balancer.on_request_end(worker_idx);
        }
    }

    /// Record a completed routing decision so cache-aware policies can update
    /// their affinity state for future requests.
    pub fn record_request(&self, ctx: &RequestContext, worker_idx: usize) {
        if self.k8s_mode {
            if let Some(ref lock) = self.k8s_balancer {
                if let Ok(policy) = lock.read() {
                    policy.record(ctx, worker_idx);
                }
            }
        } else {
            self.balancer.record(ctx, worker_idx);
        }
    }
}

// ── Router builder ─────────────────────────────────────────────────────────

/// Build the axum router with all routes and middleware.
///
/// - `/health` is publicly accessible (auth middleware bypasses it).
/// - All other routes (including the fallback proxy handler) are protected
///   by Bearer token authentication when `data_plane_api_keys` are configured.
pub fn build_router(state: Arc<AppState>, auth_config: AuthConfig) -> Router {
    Router::new()
        .route("/health", get(crate::worker::health::health_handler))
        .fallback(crate::routes::pd_proxy_handler)
        .layer(middleware::from_fn_with_state(
            auth_config,
            crate::config::auth::auth_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ── Shutdown signal ────────────────────────────────────────────────────────

/// Wait for a shutdown signal (Ctrl+C on all platforms, plus SIGTERM on Unix).
pub async fn shutdown_signal() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Ctrl+C received, shutting down");
        }
        _ = term_signal() => {
            tracing::info!("SIGTERM received, shutting down");
        }
    }
}

#[cfg(unix)]
async fn term_signal() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut sig) => {
            sig.recv().await;
        }
        Err(e) => {
            tracing::warn!("Cannot install SIGTERM handler ({}), using Ctrl+C only", e);
            future::pending::<()>().await;
        }
    }
}

#[cfg(not(unix))]
async fn term_signal() {
    future::pending::<()>().await;
}
