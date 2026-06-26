//! Core reverse-proxy handler that forwards requests to worker nodes.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http,
    response::Response,
};
use http_body_util::BodyExt;
use url::Url;

use super::context::extract_context;
use super::error::ProxyError;
use super::header::filter_hop_by_hop;
use crate::server::AppState;

/// Maximum request body size to collect in memory: 16 MB.
const MAX_BODY_SIZE: usize = 16 * 1024 * 1024;

/// RAII guard that decrements the balancer's active-request counter on drop.
struct ActiveRequest<'a> {
    state: &'a AppState,
    idx: usize,
}

impl Drop for ActiveRequest<'_> {
    fn drop(&mut self) {
        self.state.finish_request(self.idx);
    }
}

/// Handle an incoming request by forwarding it to the next worker.
///
/// # Errors
///
/// Returns [`ProxyError`] if the worker URL is invalid, the worker request fails,
/// or the response cannot be constructed.
pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response<Body>, ProxyError> {
    let (mut parts, body) = req.into_parts();

    let body_bytes = body
        .collect()
        .await
        .map_err(|e| ProxyError::BodyCollect(e.to_string()))?
        .to_bytes();

    if body_bytes.len() > MAX_BODY_SIZE {
        return Err(ProxyError::BodyCollect(
            "request body too large".to_string(),
        ));
    }

    // Extract session / prefix context for cache-aware policies.
    let ctx = extract_context(&body_bytes, &parts.headers);

    // Remove the Host header so reqwest sets it from the target URL.
    parts.headers.remove(http::header::HOST);

    let (worker_idx, worker_url) = state
        .next_worker_with_context(&ctx)
        .ok_or(ProxyError::NoHealthyWorkers)?;
    let _active = ActiveRequest {
        state: &state,
        idx: worker_idx,
    };

    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target = Url::parse(&format!(
        "{}{}",
        worker_url.trim_end_matches('/'),
        path_and_query,
    ))?;

    tracing::info!(
        method = %parts.method,
        path = %path_and_query,
        worker = %worker_url,
        "Forwarding request",
    );

    let worker_resp = state
        .client
        .request(parts.method, target.as_str())
        .headers(parts.headers)
        .body(body_bytes)
        .send()
        .await?;

    let status = worker_resp.status();
    let filtered_headers = filter_hop_by_hop(worker_resp.headers());

    let mut response_builder = Response::builder().status(status);
    for (name, value) in &filtered_headers {
        response_builder = response_builder.header(name, value);
    }

    // Record the routing decision so cache-aware policies can update affinity.
    state.record_request(&ctx, worker_idx);

    // Stream the worker response body back to the client.
    let response_body = Body::from_stream(worker_resp.bytes_stream());
    Ok(response_builder.body(response_body)?)
}
