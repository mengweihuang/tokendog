//! Binary entry point for the tokendog-router gateway.

use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use tokendog_router::{
    build_router, config::Config, policies::round_robin::RoundRobin, state::AppState,
};

/// Entry point: parse configuration, start the HTTP server, and wait for shutdown.
#[tokio::main]
async fn main() {
    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Parse configuration from CLI args and environment variables.
    let config = Config::parse();

    tracing::info!(
        listen_addr = %config.listen_addr,
        backends = ?config.backends,
        "Starting tokendog-router",
    );

    // Build application state and router.
    let state = Arc::new(AppState::new(
        config.backends,
        config.request_timeout_secs,
        RoundRobin::new(),
    ));
    let app = build_router(state);

    // Bind the TCP listener and start serving.
    let listener = TcpListener::bind(&config.listen_addr)
        .await
        .expect("Failed to bind to listen address");

    tracing::info!("Listening on {}", config.listen_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");
}

/// Wait for a shutdown signal (Ctrl+C or SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let term = {
        use tokio::signal::unix;
        let mut stream =
            unix::signal(unix::SignalKind::terminate()).expect("failed to install SIGTERM handler");
        stream.recv()
    };

    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Ctrl+C received, shutting down");
        }
        _ = term => {
            tracing::info!("SIGTERM received, shutting down");
        }
    }
}
