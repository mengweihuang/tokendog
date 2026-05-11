use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use router::policies::round_robin::RoundRobin;
use router::state::AppState;
use router::build_router;
use tower::util::ServiceExt;

#[tokio::test]
async fn test_health_endpoint_returns_ok() {
    let state = Arc::new(AppState::new(
        vec!["http://localhost:8000".to_string()],
        30,
        RoundRobin::new(),
    ));
    let app = build_router(state);

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
}

#[tokio::test]
async fn test_health_endpoint_json_body() {
    let state = Arc::new(AppState::new(
        vec![
            "http://worker1:8000".to_string(),
            "http://worker2:8000".to_string(),
        ],
        30,
        RoundRobin::new(),
    ));
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should handle health request");

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let health: serde_json::Value =
        serde_json::from_slice(&bytes).expect("health response should be valid JSON");

    assert_eq!(health["status"], "ok");
    assert_eq!(health["worker_urls"].as_array().unwrap().len(), 2);
    assert_eq!(health["worker_urls"][0], "http://worker1:8000");
    assert_eq!(health["worker_urls"][1], "http://worker2:8000");
}

#[tokio::test]
async fn test_health_endpoint_single_worker() {
    let state = Arc::new(AppState::new(
        vec!["http://single:8000".to_string()],
        30,
        RoundRobin::new(),
    ));
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should handle health request");

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let health: serde_json::Value =
        serde_json::from_slice(&bytes).expect("health response should be valid JSON");

    assert_eq!(health["status"], "ok");
    assert_eq!(health["worker_urls"][0], "http://single:8000");
}

#[tokio::test]
async fn test_health_endpoint_content_type() {
    let state = Arc::new(AppState::new(
        vec!["http://localhost:8000".to_string()],
        30,
        RoundRobin::new(),
    ));
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should handle health request");

    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("health response should have content-type");
    assert!(content_type.to_str().unwrap().contains("application/json"));
}
