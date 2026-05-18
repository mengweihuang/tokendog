//! router — LLM gateway for vLLM/SGLang inference engines.

pub mod auth;
pub mod config;
pub mod health;
pub mod policies;
pub mod proxy;
pub mod routes;
pub mod state;

use std::future;
use std::sync::Arc;

use axum::{middleware, routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::{auth::AuthConfig, state::AppState};

/// Build the axum router with all routes and middleware.
///
/// - `/health` is publicly accessible (auth middleware bypasses it).
/// - All other routes (including the fallback proxy handler) are protected
///   by Bearer token authentication when `data_plane_api_keys` are configured.
pub fn build_router(state: Arc<AppState>, auth_config: AuthConfig) -> Router {
    Router::new()
        .route("/health", get(health::health_handler))
        .fallback(routes::pd_proxy_handler)
        .layer(middleware::from_fn_with_state(auth_config, auth::auth_middleware))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

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
