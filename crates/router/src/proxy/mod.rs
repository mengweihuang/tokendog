//! Core reverse-proxy handler, error types, and header filtering.
//!
//! Forwards all incoming requests to a backend selected via round-robin,
//! streaming the response body back to the client.

pub mod error;
pub mod header;

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http,
    response::Response,
};
use http_body_util::BodyExt;
use url::Url;

use self::error::ProxyError;
use self::header::filter_hop_by_hop;
use crate::state::AppState;

/// Maximum request body size to collect in memory: 16 MB.
const MAX_BODY_SIZE: usize = 16 * 1024 * 1024;

/// Handle an incoming request by forwarding it to the next backend.
///
/// # Errors
///
/// Returns [`ProxyError`] if the backend URL is invalid, the backend request fails,
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

    // Remove the Host header so reqwest sets it from the target URL.
    parts.headers.remove(http::header::HOST);

    let backend = state.next_backend();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target = Url::parse(&format!(
        "{}{}",
        backend.trim_end_matches('/'),
        path_and_query,
    ))?;

    let backend_resp = state
        .client
        .request(parts.method, target.as_str())
        .headers(parts.headers)
        .body(body_bytes)
        .send()
        .await?;

    // Build the response, filtering out hop-by-hop headers.
    let status = backend_resp.status();
    let filtered_headers = filter_hop_by_hop(backend_resp.headers());

    let mut response_builder = Response::builder().status(status);
    for (name, value) in &filtered_headers {
        response_builder = response_builder.header(name, value);
    }

    // Stream the backend response body back to the client.
    // Using bytes_stream() + Body::from_stream() ensures SSE frames are forwarded
    // as they arrive without buffering.
    let response_body = Body::from_stream(backend_resp.bytes_stream());
    Ok(response_builder.body(response_body)?)
}
