use std::sync::Arc;

use router::policies::round_robin::RoundRobin;
use router::policies::LoadBalancer;
use router::server::AppState;
use router::worker::Worker;

fn make_workers(urls: &[&str]) -> Vec<Arc<Worker>> {
    let strings: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
    Worker::from_urls(&strings)
}

#[test]
fn test_app_state_new_with_workers() {
    let state = AppState::new(
        make_workers(&["http://localhost:8000"]),
        30,
        Box::new(RoundRobin::new()),
    );
    assert_eq!(state.workers.len(), 1);
    assert_eq!(state.workers[0].url, "http://localhost:8000");
}

#[test]
#[should_panic(expected = "AppState requires at least one worker URL")]
fn test_app_state_empty_workers_panics() {
    let _state = AppState::new(vec![], 30, Box::new(RoundRobin::new()));
}

#[test]
fn test_next_worker_returns_valid_url() {
    let state = AppState::new(
        make_workers(&["http://worker1:8000"]),
        30,
        Box::new(RoundRobin::new()),
    );
    let (_idx, worker) = state.next_worker().expect("should have a healthy worker");
    assert_eq!(worker, "http://worker1:8000");
}

#[test]
fn test_next_worker_filters_unhealthy() {
    let state = AppState::new(
        make_workers(&["http://worker1:8000", "http://worker2:8000"]),
        30,
        Box::new(RoundRobin::new()),
    );
    // Mark worker at index 0 as unhealthy.
    state.workers[0].set_healthy(false);
    // Should always select worker at index 1.
    let (idx, url) = state.next_worker().expect("should have a healthy worker");
    assert_eq!(idx, 1);
    assert_eq!(url, "http://worker2:8000");
}

#[test]
fn test_next_worker_no_healthy_workers() {
    let state = AppState::new(
        make_workers(&["http://worker1:8000"]),
        30,
        Box::new(RoundRobin::new()),
    );
    state.workers[0].set_healthy(false);
    assert!(state.next_worker().is_none());
}

#[test]
fn test_round_robin_cycles_through_workers() {
    let rr = Box::new(RoundRobin::new());
    let workers: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();

    assert_eq!(rr.select(&workers), 0);
    assert_eq!(rr.select(&workers), 1);
    assert_eq!(rr.select(&workers), 2);
    assert_eq!(rr.select(&workers), 0);
    assert_eq!(rr.select(&workers), 1);
    assert_eq!(rr.select(&workers), 2);
}

#[test]
fn test_round_robin_single_worker() {
    let rr = Box::new(RoundRobin::new());
    let workers: Vec<String> = vec!["only".to_string()];

    assert_eq!(rr.select(&workers), 0);
    assert_eq!(rr.select(&workers), 0);
    assert_eq!(rr.select(&workers), 0);
}

#[test]
fn test_round_robin_default() {
    let rr = RoundRobin::default();
    let workers: Vec<String> = vec!["a".to_string()];
    assert_eq!(rr.select(&workers), 0);
}

#[test]
fn test_next_worker_round_robin_sequence() {
    let state = AppState::new(
        make_workers(&["http://a:8000", "http://b:8000", "http://c:8000"]),
        30,
        Box::new(RoundRobin::new()),
    );

    assert_eq!(state.next_worker().unwrap().1, "http://a:8000");
    assert_eq!(state.next_worker().unwrap().1, "http://b:8000");
    assert_eq!(state.next_worker().unwrap().1, "http://c:8000");
    assert_eq!(state.next_worker().unwrap().1, "http://a:8000");
    assert_eq!(state.next_worker().unwrap().1, "http://b:8000");
}

#[tokio::test]
async fn test_next_worker_concurrent_access() {
    let state = Arc::new(AppState::new(
        make_workers(&["http://a:8000", "http://b:8000"]),
        30,
        Box::new(RoundRobin::new()),
    ));

    let mut handles = vec![];
    for _ in 0..10 {
        let s = state.clone();
        handles.push(tokio::spawn(async move {
            let _ = s.next_worker();
        }));
    }

    for h in handles {
        h.await
            .expect("concurrent next_worker calls should not panic");
    }
}
