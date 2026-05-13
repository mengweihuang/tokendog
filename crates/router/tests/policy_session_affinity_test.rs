use router::policies::session_affinity::SessionAffinity;
use router::policies::{LoadBalancer, RequestContext};

fn make_workers(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("http://worker-{}:8000", i))
        .collect()
}

#[test]
fn test_session_affinity_deterministic() {
    let policy = SessionAffinity;
    let workers = make_workers(5);
    let ctx = RequestContext {
        session_id: "user-abc".into(),
        prefix_key: String::new(),
    };

    let first = policy.select_with_context(&workers, &ctx);
    for _ in 0..100 {
        assert_eq!(policy.select_with_context(&workers, &ctx), first);
    }
}

#[test]
fn test_session_affinity_different_users_may_differ() {
    let policy = SessionAffinity;
    let workers = make_workers(100);

    let mut counts = vec![0usize; workers.len()];
    for i in 0..1000 {
        let ctx = RequestContext {
            session_id: format!("user-{}", i),
            prefix_key: String::new(),
        };
        let idx = policy.select_with_context(&workers, &ctx);
        counts[idx] += 1;
    }

    let zeros = counts.iter().filter(|&&c| c == 0).count();
    assert!(zeros < 10, "{} of 100 workers got zero traffic", zeros);
}

#[test]
fn test_session_affinity_single_worker() {
    let policy = SessionAffinity;
    let workers = make_workers(1);
    let ctx = RequestContext {
        session_id: "any-user".into(),
        prefix_key: String::new(),
    };
    assert_eq!(policy.select_with_context(&workers, &ctx), 0);
}

#[test]
fn test_session_affinity_on_request_hooks_are_noop() {
    let policy = SessionAffinity;
    // These should not panic.
    policy.on_request_start(0);
    policy.on_request_end(0);
    policy.record(
        &RequestContext {
            session_id: "s1".into(),
            prefix_key: String::new(),
        },
        0,
    );
}

#[test]
fn test_session_affinity_select_without_context() {
    let policy = SessionAffinity;
    let workers = make_workers(3);
    // Should not panic — falls back to random (or 0 for single worker).
    let idx = policy.select(&workers);
    assert!(idx < workers.len());
}
