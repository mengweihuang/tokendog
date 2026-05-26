/// Health-check endpoint for the gateway.
use axum::{extract::State, http::StatusCode, response::Json};
use serde::Serialize;
use std::sync::Arc;

use crate::state::AppState;

/// Per-worker health status in the `/health` response.
#[derive(Serialize)]
pub struct WorkerStatus {
    pub url: String,
    pub healthy: bool,
}

/// Response body for the `/health` endpoint.
#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
    workers: Vec<WorkerStatus>,
}

/// `GET /health` — Returns the gateway health status and per-worker health.
pub async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<HealthResponse>) {
    let workers: Vec<WorkerStatus> = state
        .workers
        .iter()
        .map(|w| WorkerStatus {
            url: w.url.clone(),
            healthy: w.is_healthy(),
        })
        .collect();

    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_string(),
            workers,
        }),
    )
}
