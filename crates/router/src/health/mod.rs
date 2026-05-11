/// Health-check endpoint for the gateway.
use axum::{extract::State, http::StatusCode, response::Json};
use serde::Serialize;
use std::sync::Arc;

use crate::state::AppState;

/// Response body for the `/health` endpoint.
#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
    backends: Vec<String>,
}

/// `GET /health` — Returns the gateway health status and configured backends.
pub async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<HealthResponse>) {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_string(),
            backends: state.backends.clone(),
        }),
    )
}
