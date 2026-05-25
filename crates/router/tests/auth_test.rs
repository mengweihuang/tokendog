use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use router::auth::AuthConfig;
use router::build_router;
use router::policies::round_robin::RoundRobin;
use router::state::AppState;
use router::worker::Worker;
use tower::util::ServiceExt;

/// Helper: create a minimal router app for testing.
fn test_app(auth_config: AuthConfig) -> axum::Router {
    let state = Arc::new(AppState::new(
        vec![Arc::new(Worker::new("http://localhost:8000".to_string()))],
        30,
        Box::new(RoundRobin::new()),
    ));
    build_router(state, auth_config)
}

#[tokio::test]
async fn test_auth_disabled_allows_all() {
    let app = test_app(AuthConfig::new(None));

    // Without auth header — should pass when auth is disabled
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"test"}"#))
                .unwrap(),
        )
        .await
        .expect("router should handle request");

    // Auth is disabled, so we get a proxy error (502/504) not 401
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_with_empty_keys_disables_auth() {
    let app = test_app(AuthConfig::new(Some(vec![])));

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
        .expect("router should handle request");

    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_enabled_rejects_missing_token() {
    let app = test_app(AuthConfig::new(Some(vec!["sk-valid".to_string()])));

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
        .expect("router should handle request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_enabled_rejects_wrong_token() {
    let app = test_app(AuthConfig::new(Some(vec!["sk-valid".to_string()])));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("authorization", "Bearer sk-wrong")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"test"}"#))
                .unwrap(),
        )
        .await
        .expect("router should handle request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_enabled_rejects_expired_wrong_format() {
    let app = test_app(AuthConfig::new(Some(vec!["sk-valid".to_string()])));

    // Bearer with no space after
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("authorization", "Bearer")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"test"}"#))
                .unwrap(),
        )
        .await
        .expect("router should handle request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_enabled_accepts_valid_token() {
    let app = test_app(AuthConfig::new(Some(vec!["sk-valid".to_string()])));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("authorization", "Bearer sk-valid")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"test"}"#))
                .unwrap(),
        )
        .await
        .expect("router should handle request");

    // Auth passes, but the backend is unreachable → 502/504, not 401
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response.status().is_server_error(),
        "expected 5xx from unreachable backend, got {}",
        response.status()
    );
}

#[tokio::test]
async fn test_auth_multiple_keys_all_work() {
    let app = test_app(AuthConfig::new(Some(vec![
        "sk-key1".to_string(),
        "sk-key2".to_string(),
        "sk-key3".to_string(),
    ])));

    // All valid keys should work
    for key in &["sk-key1", "sk-key2", "sk-key3"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/chat/completions")
                    .method("POST")
                    .header("authorization", format!("Bearer {}", key))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"test"}"#))
                    .unwrap(),
            )
            .await
            .expect("router should handle request");

        assert_ne!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "key '{}' should be accepted",
            key
        );
    }
}

#[tokio::test]
async fn test_health_endpoint_bypasses_auth() {
    let app = test_app(AuthConfig::new(Some(vec!["sk-secret".to_string()])));

    // Health check without any auth token should still work
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should handle health request");

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let health: serde_json::Value =
        serde_json::from_slice(&bytes).expect("health response should be valid JSON");
    assert_eq!(health["status"], "ok");
}
