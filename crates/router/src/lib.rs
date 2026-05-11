//! tokendog-router — Round-robin HTTP gateway for vLLM/SGLang inference engines.

pub mod config;
pub mod health;
pub mod policies;
pub mod proxy;
pub mod state;

use std::sync::Arc;

use axum::{routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::{proxy::proxy_handler, state::AppState};

/// Build the axum router with all routes and middleware.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health::health_handler))
        .fallback(proxy_handler)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
