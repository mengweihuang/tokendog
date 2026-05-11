//! Binary entry point for the router gateway.

use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use router::{
    build_router, config::Config, policies::round_robin::RoundRobin, shutdown_signal,
    state::AppState,
};

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

    tracing::info!(
        host = %config.host,
        port = config.port,
        worker_urls = ?config.worker_urls,
        "Starting router",
    );

    // Build application state and router.
    let state = Arc::new(AppState::new(
        config.worker_urls,
        config.request_timeout_secs,
        RoundRobin::new(),
    ));
    let app = build_router(state);

    // Bind the TCP listener and start serving.
    let listener = TcpListener::bind(&listen_addr)
        .await
        .expect("Failed to bind to listen address");

    tracing::info!("Listening on {}", listen_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");
}
