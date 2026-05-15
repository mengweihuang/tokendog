use std::sync::Arc;

use router::policies::round_robin::RoundRobin;
use router::policies::LoadBalancer;
use router::state::AppState;

#[test]
fn test_app_state_new_with_workers() {
    let state = AppState::new(
        vec!["http://localhost:8000".to_string()],
        30,
        Box::new(RoundRobin::new()),
    );
    assert_eq!(state.worker_urls.len(), 1);
    assert_eq!(state.worker_urls[0], "http://localhost:8000");
}

#[test]
#[should_panic(expected = "AppState requires at least one worker URL")]
fn test_app_state_empty_workers_panics() {
    let _state = AppState::new(vec![], 30, Box::new(RoundRobin::new()));
}

#[test]
fn test_next_worker_returns_valid_url() {
    let state = AppState::new(
        vec!["http://worker1:8000".to_string()],
        30,
        Box::new(RoundRobin::new()),
    );
    let (_idx, worker) = state.next_worker();
    assert_eq!(worker, "http://worker1:8000");
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
        vec![
            "http://a:8000".to_string(),
            "http://b:8000".to_string(),
            "http://c:8000".to_string(),
        ],
        30,
        Box::new(RoundRobin::new()),
    );

    assert_eq!(state.next_worker().1, "http://a:8000");
    assert_eq!(state.next_worker().1, "http://b:8000");
    assert_eq!(state.next_worker().1, "http://c:8000");
    assert_eq!(state.next_worker().1, "http://a:8000");
    assert_eq!(state.next_worker().1, "http://b:8000");
}

#[tokio::test]
async fn test_next_worker_concurrent_access() {
    let state = Arc::new(AppState::new(
        vec!["http://a:8000".to_string(), "http://b:8000".to_string()],
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
