//! Prefill-Decode proxy handler for vLLM and SGLang disaggregated inference.
//!
//! When PD mode is active, inference API requests go through a two-stage pipeline:
//! 1. **Prefill** — send a `max_tokens=1` variant to a prefill worker to populate the KV cache.
//! 2. **Decode** — forward the original request plus KV transfer params to a decode worker,
//!    streaming the response back to the client.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use serde_json::Value;
use thiserror::Error;
use tracing::info;
use url::Url;

use super::{logprobs_merge, prefill};
use crate::middleware::{context::extract_context, header::filter_hop_by_hop};
use crate::server::AppState;

/// Maximum request body size: 16 MB.
const MAX_BODY_SIZE: usize = 16 * 1024 * 1024;

// ── Error types ────────────────────────────────────────────────────────────

/// Errors that can occur during PD two-stage request processing.
#[derive(Debug, Error)]
enum PdProxyError {
    #[error("Failed to read request body: {0}")]
    BodyCollect(String),
    #[error("Failed to parse request JSON: {0}")]
    JsonParse(String),
    #[error("No {0} workers available")]
    NoWorkers(String),
    #[error("Prefill request to {0} failed: {1}")]
    PrefillRequest(String, String),
    #[error("Prefill returned {0}: {1}")]
    PrefillError(StatusCode, String),
    #[error("Failed to read prefill response: {0}")]
    PrefillResponse(String),
    #[error("Failed to parse prefill response JSON: {0}")]
    PrefillParse(String),
    #[error("Decode request to {0} failed: {1}")]
    DecodeRequest(String, String),
    #[error("Failed to read decode response: {0}")]
    DecodeResponse(String),
    #[error("Failed to parse decode response JSON: {0}")]
    DecodeParse(String),
    #[error("Failed to serialize merged response: {0}")]
    JsonSerialize(String),
    #[error(transparent)]
    WorkerRequest(#[from] reqwest::Error),
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
    #[error(transparent)]
    ResponseBuild(#[from] axum::http::Error),
}

impl IntoResponse for PdProxyError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            PdProxyError::BodyCollect(m) => (StatusCode::BAD_REQUEST, m.clone()),
            PdProxyError::JsonParse(m) => (StatusCode::BAD_REQUEST, m.clone()),
            PdProxyError::NoWorkers(m) => (StatusCode::SERVICE_UNAVAILABLE, m.clone()),
            PdProxyError::PrefillRequest(url, msg) => (
                StatusCode::BAD_GATEWAY,
                format!("Prefill request to {url} failed: {msg}"),
            ),
            PdProxyError::PrefillError(s, msg) => (*s, msg.clone()),
            PdProxyError::PrefillResponse(m) => (
                StatusCode::BAD_GATEWAY,
                format!("Failed to read prefill response: {m}"),
            ),
            PdProxyError::PrefillParse(m) => (
                StatusCode::BAD_GATEWAY,
                format!("Failed to parse prefill response JSON: {m}"),
            ),
            PdProxyError::DecodeRequest(url, msg) => (
                StatusCode::BAD_GATEWAY,
                format!("Decode request to {url} failed: {msg}"),
            ),
            PdProxyError::DecodeResponse(m) => (
                StatusCode::BAD_GATEWAY,
                format!("Failed to read decode response: {m}"),
            ),
            PdProxyError::DecodeParse(m) => (
                StatusCode::BAD_GATEWAY,
                format!("Failed to parse decode response JSON: {m}"),
            ),
            PdProxyError::JsonSerialize(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.clone()),
            PdProxyError::WorkerRequest(e) => {
                if e.is_timeout() {
                    (StatusCode::GATEWAY_TIMEOUT, e.to_string())
                } else {
                    (StatusCode::BAD_GATEWAY, e.to_string())
                }
            }
            PdProxyError::UrlParse(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            PdProxyError::ResponseBuild(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        let body = Body::from(msg);
        Response::builder()
            .status(status)
            .body(body)
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("Failed to build error response"))
                    .unwrap()
            })
    }
}

/// Generate a short request ID for correlating prefill and decode in logs.
fn generate_request_id() -> String {
    format!("pd-{:016x}", rand::random::<u64>())
}

/// Returns `true` if `path` targets a known LLM inference endpoint.
fn is_inference_path(path: &str) -> bool {
    path.contains("/v1/chat/completions")
        || path.contains("/v1/completions")
        || path.contains("/v1/responses")
        || path.contains("/inference/v1/generate")
}

// ── Main handler ───────────────────────────────────────────────────────────

/// Universal fallback handler — PD two-stage pipeline for inference paths when
/// PD mode is active, otherwise delegates to the regular proxy handler.
pub async fn pd_proxy_handler(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path().to_string();

    match state.pd_mode() {
        Some(mode) if is_inference_path(&path) => {
            let collected = match body.collect().await {
                Ok(c) => c.to_bytes(),
                Err(e) => {
                    return PdProxyError::BodyCollect(e.to_string()).into_response();
                }
            };

            if collected.len() > MAX_BODY_SIZE {
                return PdProxyError::BodyCollect("request body too large".to_string())
                    .into_response();
            }
            let body_bytes = collected;

            let request_json: Value = match serde_json::from_slice(&body_bytes) {
                Ok(v) => v,
                Err(e) => return PdProxyError::JsonParse(e.to_string()).into_response(),
            };
            let ctx = extract_context(&body_bytes, &parts.headers);

            let result = match mode {
                crate::config::PdMode::Vllm => {
                    process_vllm_pd_request(&state, &path, request_json, &ctx, &parts.headers).await
                }
                crate::config::PdMode::Sglang => {
                    process_sglang_pd_request(&state, &path, request_json, &ctx, &parts.headers)
                        .await
                }
            };
            match result {
                Ok(response) => response,
                Err(e) => e.into_response(),
            }
        }
        _ => {
            let forward_req = Request::from_parts(parts, body);
            match crate::middleware::proxy_handler(State(state), forward_req).await {
                Ok(response) => response,
                Err(e) => e.into_response(),
            }
        }
    }
}

// ── Two-stage pipeline ─────────────────────────────────────────────────────

/// SGLang PD: concurrent dual dispatch.
///
/// Both prefill and decode requests are sent simultaneously with
/// `bootstrap_host`, `bootstrap_port`, and `bootstrap_room` injected
/// into each request body. The decode response is returned to the client.
async fn process_sglang_pd_request(
    state: &AppState,
    path: &str,
    request_json: Value,
    ctx: &crate::policies::RequestContext,
    request_headers: &axum::http::HeaderMap,
) -> Result<Response, PdProxyError> {
    let request_id = generate_request_id();

    // ── Select workers ──────────────────────────────────────────────────

    let (prefill_idx, prefill_url) = state
        .next_prefill_worker(ctx)
        .ok_or_else(|| PdProxyError::NoWorkers("prefill".to_string()))?;
    let (decode_idx, decode_url) = state
        .next_decode_worker(ctx)
        .ok_or_else(|| PdProxyError::NoWorkers("decode".to_string()))?;

    let (bootstrap_host, bootstrap_port, bootstrap_room) =
        prefill::build_sglang_bootstrap_params(prefill_url, prefill::DEFAULT_BOOTSTRAP_PORT);

    let si = detect_streaming_info(&request_json);

    // ── Build prefill request with bootstrap params ─────────────────────

    let mut prefill_request = request_json.clone();
    if let Some(obj) = prefill_request.as_object_mut() {
        obj.insert(
            "bootstrap_host".to_string(),
            Value::String(bootstrap_host.clone()),
        );
        obj.insert(
            "bootstrap_port".to_string(),
            serde_json::json!(bootstrap_port),
        );
        obj.insert(
            "bootstrap_room".to_string(),
            serde_json::json!(bootstrap_room),
        );
    }

    // ── Build decode request with bootstrap params ──────────────────────

    let mut decode_request = request_json.clone();
    if let Some(obj) = decode_request.as_object_mut() {
        obj.insert(
            "bootstrap_host".to_string(),
            Value::String(bootstrap_host),
        );
        obj.insert(
            "bootstrap_port".to_string(),
            serde_json::json!(bootstrap_port),
        );
        obj.insert(
            "bootstrap_room".to_string(),
            serde_json::json!(bootstrap_room),
        );
    }

    let prefill_target = build_target_url(prefill_url, path)?;
    let decode_target = build_target_url(decode_url, path)?;

    info!(
        request_id = %request_id,
        prefill_url = %prefill_target,
        decode_url = %decode_target,
        bootstrap_room = bootstrap_room,
        "SGLang PD: concurrent dual dispatch",
    );

    // ── Concurrent dispatch ─────────────────────────────────────────────

    let prefill_fut = state
        .client
        .post(prefill_target.as_str())
        .headers(filter_headers(request_headers))
        .json(&prefill_request)
        .send();

    let decode_fut = state
        .client
        .post(decode_target.as_str())
        .headers(filter_headers(request_headers))
        .json(&decode_request)
        .send();

    let (prefill_resp, decode_resp) =
        tokio::try_join!(prefill_fut, decode_fut).map_err(PdProxyError::WorkerRequest)?;

    state.finish_prefill_request(prefill_idx);
    state.record_prefill_request(ctx, prefill_idx);
    state.finish_decode_request(decode_idx);
    state.record_decode_request(ctx, decode_idx);

    // ── Process prefill (for logprobs) ──────────────────────────────────

    let prefill_json: Option<Value> = if si.needs_logprobs && !si.is_streaming {
        match prefill_resp.bytes().await {
            Ok(bytes) => serde_json::from_slice(&bytes).ok(),
            Err(_) => None,
        }
    } else {
        None
    };

    // ── Build response from decode ──────────────────────────────────────

    let status = decode_resp.status();
    let decode_headers = filter_hop_by_hop(decode_resp.headers());

    if si.is_streaming {
        return build_response(
            status,
            &decode_headers,
            Body::from_stream(decode_resp.bytes_stream()),
        );
    }

    let decode_bytes = decode_resp
        .bytes()
        .await
        .map_err(|e| PdProxyError::DecodeResponse(e.to_string()))?;

    if si.needs_logprobs {
        if let Ok(mut decode_json) = serde_json::from_slice::<Value>(&decode_bytes) {
            if let Some(ref prefill) = prefill_json {
                logprobs_merge::merge_logprobs_in_json(prefill, &mut decode_json);
            }
            let merged = serde_json::to_vec(&decode_json)
                .map_err(|e| PdProxyError::JsonSerialize(e.to_string()))?;
            return build_response(status, &decode_headers, Body::from(merged));
        }
    }

    build_response(status, &decode_headers, Body::from(decode_bytes))
}

/// vLLM PD: sequential two-stage processing.
///
/// Prefill first with `max_tokens=1`, then decode with `kv_transfer_params`
/// from the prefill response.
async fn process_vllm_pd_request(
    state: &AppState,
    path: &str,
    request_json: Value,
    ctx: &crate::policies::RequestContext,
    request_headers: &axum::http::HeaderMap,
) -> Result<Response, PdProxyError> {
    let request_id = generate_request_id();

    // ── Stage 1: Prefill ───────────────────────────────────────────────

    let (prefill_idx, prefill_url) = state
        .next_prefill_worker(ctx)
        .ok_or_else(|| PdProxyError::NoWorkers("prefill".to_string()))?;

    let mut prefill_request = prefill::prepare_prefill_request(request_json.clone(), path);
    prefill_request["kv_transfer_params"] = prefill::build_prefill_kv_transfer_params();

    // Clone for decode stage before the prefill network call, so the
    // allocation can overlap with the round-trip.
    let decode_request = request_json.clone();

    let prefill_target = build_target_url(prefill_url, path)?;

    info!(
        request_id = %request_id,
        prefill_url = %prefill_target,
        "PD stage 1: sending prefill request",
    );

    let prefill_resp = state
        .client
        .post(prefill_target.as_str())
        .headers(filter_headers(request_headers))
        .json(&prefill_request)
        .send()
        .await
        .map_err(|e| PdProxyError::PrefillRequest(prefill_url.to_string(), e.to_string()))?;

    let prefill_status = prefill_resp.status();
    if !prefill_status.is_success() {
        state.finish_prefill_request(prefill_idx);
        let error_body = prefill_resp.text().await.unwrap_or_default();
        return Err(PdProxyError::PrefillError(prefill_status, error_body));
    }

    let prefill_bytes = prefill_resp
        .bytes()
        .await
        .map_err(|e| PdProxyError::PrefillResponse(e.to_string()))?;

    let prefill_json: Value = serde_json::from_slice(&prefill_bytes)
        .map_err(|e| PdProxyError::PrefillParse(e.to_string()))?;

    state.finish_prefill_request(prefill_idx);
    state.record_prefill_request(ctx, prefill_idx);

    let kv_transfer_params = prefill::build_decode_kv_transfer_params(&prefill_json);

    // ── Stage 2: Decode ────────────────────────────────────────────────

    let (decode_idx, decode_url) = state
        .next_decode_worker(ctx)
        .ok_or_else(|| PdProxyError::NoWorkers("decode".to_string()))?;

    let mut decode_request = decode_request;
    if let Some(ref params) = kv_transfer_params {
        if let Some(obj) = decode_request.as_object_mut() {
            obj.insert("kv_transfer_params".to_string(), params.clone());
        }
    }

    let si = detect_streaming_info(&request_json);

    let decode_target = build_target_url(decode_url, path)?;

    info!(
        request_id = %request_id,
        decode_url = %decode_target,
        streaming = si.is_streaming,
        "PD stage 2: sending decode request",
    );

    let decode_resp = state
        .client
        .post(decode_target.as_str())
        .headers(filter_headers(request_headers))
        .json(&decode_request)
        .send()
        .await
        .map_err(|e| PdProxyError::DecodeRequest(decode_url.to_string(), e.to_string()))?;

    state.finish_decode_request(decode_idx);
    state.record_decode_request(ctx, decode_idx);

    let status = decode_resp.status();
    let decode_headers = filter_hop_by_hop(decode_resp.headers());

    // ── Build response ─────────────────────────────────────────────────

    if si.is_streaming {
        return build_response(
            status,
            &decode_headers,
            Body::from_stream(decode_resp.bytes_stream()),
        );
    }

    let decode_bytes = decode_resp
        .bytes()
        .await
        .map_err(|e| PdProxyError::DecodeResponse(e.to_string()))?;

    if si.needs_logprobs {
        let mut decode_json: Value = serde_json::from_slice(&decode_bytes)
            .map_err(|e| PdProxyError::DecodeParse(e.to_string()))?;
        logprobs_merge::merge_logprobs_in_json(&prefill_json, &mut decode_json);
        let merged = serde_json::to_vec(&decode_json)
            .map_err(|e| PdProxyError::JsonSerialize(e.to_string()))?;
        return build_response(status, &decode_headers, Body::from(merged));
    }

    build_response(status, &decode_headers, Body::from(decode_bytes))
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn build_target_url(worker_url: &str, path: &str) -> Result<Url, url::ParseError> {
    Url::parse(&format!("{}{}", worker_url.trim_end_matches('/'), path))
}

/// Copy request headers, removing `Host`.
fn filter_headers(headers: &axum::http::HeaderMap) -> axum::http::HeaderMap {
    let mut filtered = headers.clone();
    filtered.remove(axum::http::header::HOST);
    filtered
}

struct StreamingInfo {
    is_streaming: bool,
    needs_logprobs: bool,
}

fn detect_streaming_info(request_json: &Value) -> StreamingInfo {
    StreamingInfo {
        is_streaming: request_json
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        needs_logprobs: request_json.get("logprobs").is_some()
            || request_json
                .get("echo")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
    }
}

fn build_response(
    status: StatusCode,
    headers: &[(axum::http::HeaderName, axum::http::HeaderValue)],
    body: Body,
) -> Result<Response, PdProxyError> {
    let mut response_builder = Response::builder().status(status);
    for (name, value) in headers {
        if name.as_str() != "content-length" {
            response_builder = response_builder.header(name, value);
        }
    }
    Ok(response_builder.body(body)?)
}
