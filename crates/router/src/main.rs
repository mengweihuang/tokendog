//! Binary entry point for the router gateway.

use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use router::{
    auth::AuthConfig, build_router, config::{self, Config},
    policies::{
        least_loaded::LeastLoaded, load_cache_aware::LoadCacheAware, power_of_two::PowerOfTwo,
        prefix_affinity::PrefixAffinity, random::Random, round_robin::RoundRobin,
        session_affinity::SessionAffinity, LoadBalancer,
    },
    shutdown_signal,
    state::AppState,
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
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let listen_addr = format!("{}:{}", config.host, config.port);

    let num_api_keys = config.data_plane_api_keys.len();
    let auth_config = AuthConfig::new(Some(config.data_plane_api_keys));

    tracing::info!(
        host = %config.host,
        port = config.port,
        worker_urls = ?config.worker_urls,
        prefill_urls = ?config.prefill_urls,
        decode_urls = ?config.decode_urls,
        pd_mode = ?config.pd_mode,
        policy = ?config.policy,
        data_plane_auth = num_api_keys > 0,
        "Starting router",
    );

    // Build application state with the selected load-balancing policy.
    let state: Arc<AppState> = if let Some(pd_mode) = config.pd_mode {
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
            config.prefill_urls,
            config.decode_urls,
            config.request_timeout_secs,
            make_policy(config.policy, n_prefill),
            make_policy(config.policy, n_decode),
        ))
    } else {
        let n = config.worker_urls.len();
        Arc::new(AppState::new(
            config.worker_urls,
            config.request_timeout_secs,
            make_policy(config.policy, n),
        ))
    };
    let app = build_router(state, auth_config);
    let listener = TcpListener::bind(&listen_addr)
        .await
        .expect("Failed to bind to listen address");

    tracing::info!("Listening on {}", listen_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");
}
