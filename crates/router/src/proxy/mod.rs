//! Core reverse-proxy handler, error types, and header filtering.
//!
//! Forwards all incoming requests to a worker selected via the configured
//! load-balancing policy, streaming the response body back to the client.

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
use crate::policies::RequestContext;
use crate::state::AppState;

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

/// Extract [`RequestContext`] from the raw request body bytes.
///
/// Parses the JSON body to pull out:
/// - `session_id`: from `"user"` field, falling back to `"session_id"`,
///   then to `"default"`.
/// - `prefix_key`: first 200 characters of the first message's `"content"`,
///   preferring `system`-role messages, defaulting to `"default"`.
pub(crate) fn extract_context(body: &[u8]) -> RequestContext {
    let default = RequestContext {
        session_id: "default".to_string(),
        prefix_key: "default".to_string(),
    };

    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return default,
    };

    let session_id = v
        .get("user")
        .or_else(|| v.get("session_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string());

    let prefix_key = v
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|msgs| {
            // Prefer the first system-role message.
            msgs.iter()
                .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
                .or_else(|| msgs.first())
        })
        .and_then(|msg| msg.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| {
            // Take up to 200 chars (clamp at a char boundary).
            s.char_indices()
                .take(200)
                .last()
                .map(|(idx, c)| &s[..idx + c.len_utf8()])
                .unwrap_or(s)
                .to_string()
        })
        .unwrap_or_else(|| "default".to_string());

    RequestContext {
        session_id,
        prefix_key,
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
    let ctx = extract_context(&body_bytes);

    // Remove the Host header so reqwest sets it from the target URL.
    parts.headers.remove(http::header::HOST);

    let (worker_idx, worker_url) = state.next_worker_with_context(&ctx);
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

    // Build the response, filtering out hop-by-hop headers.
    let status = worker_resp.status();
    let filtered_headers = filter_hop_by_hop(worker_resp.headers());

    let mut response_builder = Response::builder().status(status);
    for (name, value) in &filtered_headers {
        response_builder = response_builder.header(name, value);
    }

    // Record the routing decision so cache-aware policies can update affinity.
    state.record_request(&ctx, worker_idx);

    // Stream the worker response body back to the client.
    // Using bytes_stream() + Body::from_stream() ensures SSE frames are forwarded
    // as they arrive without buffering.
    let response_body = Body::from_stream(worker_resp.bytes_stream());
    Ok(response_builder.body(response_body)?)
}
