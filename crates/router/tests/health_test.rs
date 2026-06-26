use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use router::config::auth::AuthConfig;
use router::policies::round_robin::RoundRobin;
use router::server::{self, AppState};
use router::worker::Worker;
use tower::util::ServiceExt;

fn make_workers(urls: &[&str]) -> Vec<Arc<Worker>> {
    let strings: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
    Worker::from_urls(&strings)
}

#[tokio::test]
async fn test_health_endpoint_returns_ok() {
    let state = Arc::new(AppState::new(
        make_workers(&["http://localhost:8000"]),
        30,
        Box::new(RoundRobin::new()),
    ));
    let app = server::build_router(state, AuthConfig::new(None));

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
        make_workers(&["http://worker1:8000", "http://worker2:8000"]),
        30,
        Box::new(RoundRobin::new()),
    ));
    let app = server::build_router(state, AuthConfig::new(None));

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
    assert_eq!(health["workers"].as_array().unwrap().len(), 2);
    assert_eq!(health["workers"][0]["url"], "http://worker1:8000");
    assert_eq!(health["workers"][0]["healthy"], true);
    assert_eq!(health["workers"][1]["url"], "http://worker2:8000");
    assert_eq!(health["workers"][1]["healthy"], true);
}

#[tokio::test]
async fn test_health_endpoint_single_worker() {
    let state = Arc::new(AppState::new(
        make_workers(&["http://single:8000"]),
        30,
        Box::new(RoundRobin::new()),
    ));
    let app = server::build_router(state, AuthConfig::new(None));

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
    assert_eq!(health["workers"][0]["url"], "http://single:8000");
    assert_eq!(health["workers"][0]["healthy"], true);
}

#[tokio::test]
async fn test_health_endpoint_content_type() {
    let state = Arc::new(AppState::new(
        make_workers(&["http://localhost:8000"]),
        30,
        Box::new(RoundRobin::new()),
    ));
    let app = server::build_router(state, AuthConfig::new(None));

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

#[test]
fn test_worker_starts_healthy() {
    let worker = Worker::new("http://localhost:8000".to_string());
    assert!(worker.is_healthy());
    assert_eq!(worker.url, "http://localhost:8000");
}

#[test]
fn test_worker_health_transition() {
    let worker = Worker::new("http://localhost:8000".to_string());
    assert!(worker.is_healthy());
    worker.set_healthy(false);
    assert!(!worker.is_healthy());
    worker.set_healthy(true);
    assert!(worker.is_healthy());
}
