//! Binary entry point for the router gateway.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use clap::Parser;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use router::{
    config::{self, auth::AuthConfig, Config, parse_selector},
    policies::{
        least_loaded::LeastLoaded, load_cache_aware::LoadCacheAware, power_of_two::PowerOfTwo,
        prefix_affinity::PrefixAffinity, random::Random, round_robin::RoundRobin,
        session_affinity::SessionAffinity, LoadBalancer,
    },
    server::{self, AppState},
    service_discovery::{self, ServiceDiscoveryConfig, SharedWorkerPool},
    worker::{self, Worker},
};

/// Helper: construct a `LoadBalancer` implementation from the chosen policy
/// and worker count.
fn make_policy(policy: config::Policy, n: usize) -> Box<dyn LoadBalancer> {
    match policy {
        config::Policy::LeastLoaded => Box::new(LeastLoaded::new(n)),
        config::Policy::PowerOfTwo => Box::new(PowerOfTwo::new(n)),
        config::Policy::Random => Box::new(Random),
        config::Policy::RoundRobin => Box::new(RoundRobin::new()),
        config::Policy::SessionAffinity => Box::new(SessionAffinity),
        config::Policy::PrefixAffinity => Box::new(PrefixAffinity::new(n)),
        config::Policy::LoadCacheAware => Box::new(LoadCacheAware::new(n)),
    }
}

/// Entry point: parse configuration, start the HTTP server, and wait for shutdown.
#[tokio::main]
async fn main() {
    // Parse configuration from CLI args and environment variables (do this first
    // so --log-level can influence the tracing filter).
    let config = Config::parse();

    // Initialize structured logging. RUST_LOG from the environment takes
    // precedence; otherwise the --log-level argument (or its default) is used.
    let env_filter = EnvFilter::builder()
        .with_default_directive(config.log_level.to_tracing_level().into())
        .from_env_lossy();

    router::config::logging::setup_tracing(env_filter, config.log_file.as_deref())
        .expect("Failed to initialize tracing subscriber");

    let listen_addr = format!("{}:{}", config.host, config.port);

    let num_api_keys = config.data_plane_api_keys.len();
    let is_k8s = config.has_k8s_enabled();
    let auth_config = AuthConfig::new(Some(config.data_plane_api_keys));

    // ── Build application state (static or K8s mode) ─────────────────────
    let state: Arc<AppState> = if is_k8s {
        tracing::info!(
            host = %config.host,
            port = config.port,
            k8s_namespace = ?config.k8s_namespace,
            k8s_selector = ?config.k8s_selector,
            k8s_prefill_selector = ?config.k8s_prefill_selector,
            k8s_decode_selector = ?config.k8s_decode_selector,
            k8s_port = config.k8s_port,
            k8s_check_interval_secs = config.k8s_check_interval_secs,
            pd_mode = ?config.pd_mode,
            policy = ?config.policy,
            health_check = config.health_check,
            health_check_interval_secs = config.health_check_interval_secs,
            data_plane_auth = num_api_keys > 0,
            "Starting router (K8s service discovery mode)",
        );

        if let Some(pd_mode) = config.pd_mode {
            let prefill_pool: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));
            let decode_pool: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));

            let state = Arc::new(AppState::new_k8s_pd(
                pd_mode,
                config.request_timeout_secs,
                config.policy,
                Arc::clone(&prefill_pool),
                Arc::clone(&decode_pool),
            ));

            // Build service discovery config for PD mode.
            let sd_config = ServiceDiscoveryConfig {
                enabled: true,
                selector: parse_selector(&config.k8s_selector),
                check_interval: Duration::from_secs(config.k8s_check_interval_secs),
                port: config.k8s_port,
                namespace: config.k8s_namespace.clone(),
                pd_mode: true,
                prefill_selector: parse_selector(&config.k8s_prefill_selector),
                decode_selector: parse_selector(&config.k8s_decode_selector),
            };

            // Start service discovery.
            match service_discovery::start_service_discovery(
                sd_config,
                Arc::new(RwLock::new(Vec::new())), // regular pool (unused in PD mode)
                Some(prefill_pool),
                Some(decode_pool),
            )
            .await
            {
                Ok(handle) => {
                    tracing::info!("K8s service discovery started");
                    tokio::spawn(async move {
                        if let Err(e) = handle.await {
                            tracing::error!("Service discovery task failed: {:?}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to start service discovery: {e}");
                    tracing::warn!("Continuing without service discovery");
                }
            }

            state
        } else {
            let workers_pool: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));

            let state = Arc::new(AppState::new_k8s(
                config.request_timeout_secs,
                config.policy,
                Arc::clone(&workers_pool),
            ));

            // Build service discovery config for regular mode.
            let sd_config = ServiceDiscoveryConfig {
                enabled: true,
                selector: parse_selector(&config.k8s_selector),
                check_interval: Duration::from_secs(config.k8s_check_interval_secs),
                port: config.k8s_port,
                namespace: config.k8s_namespace.clone(),
                pd_mode: false,
                prefill_selector: Default::default(),
                decode_selector: Default::default(),
            };

            // Start service discovery.
            match service_discovery::start_service_discovery(
                sd_config,
                workers_pool,
                None,
                None,
            )
            .await
            {
                Ok(handle) => {
                    tracing::info!("K8s service discovery started");
                    tokio::spawn(async move {
                        if let Err(e) = handle.await {
                            tracing::error!("Service discovery task failed: {:?}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to start service discovery: {e}");
                    tracing::warn!("Continuing without service discovery");
                }
            }

            state
        }
    } else {
        // ── Static worker mode (original behavior) ────────────────────────
        tracing::info!(
            host = %config.host,
            port = config.port,
            worker_urls = ?config.worker_urls,
            prefill_urls = ?config.prefill_urls,
            decode_urls = ?config.decode_urls,
            pd_mode = ?config.pd_mode,
            policy = ?config.policy,
            health_check = config.health_check,
            health_check_interval_secs = config.health_check_interval_secs,
            data_plane_auth = num_api_keys > 0,
            "Starting router (static worker mode)",
        );

        if let Some(pd_mode) = config.pd_mode {
            assert!(
                !config.prefill_urls.is_empty(),
                "PD mode requires at least one --prefill-urls"
            );
            assert!(
                !config.decode_urls.is_empty(),
                "PD mode requires at least one --decode-urls"
            );

            let n_prefill = config.prefill_urls.len();
            let n_decode = config.decode_urls.len();

            Arc::new(AppState::new_pd(
                pd_mode,
                Worker::from_urls(&config.prefill_urls),
                Worker::from_urls(&config.decode_urls),
                config.request_timeout_secs,
                make_policy(config.policy, n_prefill),
                make_policy(config.policy, n_decode),
            ))
        } else {
            let n = config.worker_urls.len();
            Arc::new(AppState::new(
                Worker::from_urls(&config.worker_urls),
                config.request_timeout_secs,
                make_policy(config.policy, n),
            ))
        }
    };

    // ── Health check task ─────────────────────────────────────────────────
    if config.health_check {
        let interval = Duration::from_secs(config.health_check_interval_secs);
        let health_client = state.client.clone();
        let state_ref = Arc::clone(&state);

        tokio::spawn(async move {
            tracing::info!(
                interval_secs = interval.as_secs(),
                "Health check task started",
            );
            loop {
                tokio::time::sleep(interval).await;

                if state_ref.is_k8s_mode() {
                    // In K8s mode, workers are in RwLock pools — take snapshots.
                    let regular = state_ref.k8s_workers_snapshot();
                    let prefill = state_ref.k8s_prefill_snapshot();
                    let decode = state_ref.k8s_decode_snapshot();

                    if !regular.is_empty() {
                        worker::run_health_checks(&health_client, &regular).await;
                    }
                    if !prefill.is_empty() {
                        worker::run_health_checks(&health_client, &prefill).await;
                    }
                    if !decode.is_empty() {
                        worker::run_health_checks(&health_client, &decode).await;
                    }
                } else {
                    // Static mode — use original vectors.
                    worker::run_health_checks(&health_client, &state_ref.workers).await;
                    let prefill = state_ref.prefill_workers();
                    let decode = state_ref.decode_workers();
                    if !prefill.is_empty() {
                        worker::run_health_checks(&health_client, prefill).await;
                    }
                    if !decode.is_empty() {
                        worker::run_health_checks(&health_client, decode).await;
                    }
                }
            }
        });
    }

    let app = server::build_router(state, auth_config);
    let listener = TcpListener::bind(&listen_addr)
        .await
        .expect("Failed to bind to listen address");

    tracing::info!("Listening on {}", listen_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(server::shutdown_signal())
        .await
        .expect("Server error");
}
