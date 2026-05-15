use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use router::build_router;
use router::policies::round_robin::RoundRobin;
use router::state::AppState;
use tower::util::ServiceExt;

#[tokio::test]
async fn test_proxy_handler_backend_unreachable() {
    let state = Arc::new(AppState::new(
        // Use a port that is very unlikely to have a listening service.
        vec!["http://127.0.0.1:1".to_string()],
        3,
        Box::new(RoundRobin::new()),
    ));
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"test"}"#))
                .unwrap(),
        )
        .await
        .expect("router should handle proxy request");

    // Connection refused should map to 502 BAD_GATEWAY.
    assert!(
        response.status() == StatusCode::BAD_GATEWAY
            || response.status() == StatusCode::GATEWAY_TIMEOUT,
        "expected 502 or 504, got {}",
        response.status()
    );
}

#[tokio::test]
async fn test_proxy_handler_invalid_worker_url() {
    let state = Arc::new(AppState::new(
        vec!["http://[::1]:8000".to_string()],
        3,
        Box::new(RoundRobin::new()),
    ));
    let app = build_router(state);

    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .expect("router should handle proxy request");

    // An actual connection attempt may either succeed (if IPv6 localhost
    // has something listening) or fail. This test validates the router
    // doesn't panic and returns a proper HTTP response.
    assert!(
        response.status().is_server_error() || response.status().is_success(),
        "unexpected status: {}",
        response.status()
    );
}
