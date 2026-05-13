use router::policies::prefix_affinity::PrefixAffinity;
use router::policies::{LoadBalancer, RequestContext};

fn make_workers(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("http://worker-{}:8000", i))
        .collect()
}

#[test]
fn test_prefix_affinity_same_prefix_same_worker_under_threshold() {
    let policy = PrefixAffinity::new(5);
    let workers = make_workers(5);
    let ctx = RequestContext {
        session_id: "s1".into(),
        prefix_key: "You are a helpful assistant. Always respond in JSON.".into(),
    };

    let first = policy.select_with_context(&workers, &ctx);
    for _ in 0..10 {
        assert_eq!(policy.select_with_context(&workers, &ctx), first);
    }
}

#[test]
fn test_prefix_affinity_fallback_when_overloaded() {
    let policy = PrefixAffinity::with_threshold(3, 2);
    let workers = make_workers(3);
    let ctx = RequestContext {
        session_id: "s1".into(),
        prefix_key: "system-prompt-a".into(),
    };

    let preferred = policy.select_with_context(&workers, &ctx);

    // Saturate the preferred worker.
    policy.on_request_start(preferred);
    policy.on_request_start(preferred);

    // Now preferred is at threshold (2), should fall back.
    let fallback = policy.select_with_context(&workers, &ctx);
    assert_ne!(fallback, preferred);

    policy.on_request_end(preferred);
    policy.on_request_end(preferred);
}

#[test]
fn test_prefix_affinity_different_prefixes_distribute() {
    let policy = PrefixAffinity::new(10);
    let workers = make_workers(10);

    let mut counts = [0usize; 10];
    for i in 0..1000 {
        let ctx = RequestContext {
            session_id: format!("s-{}", i),
            prefix_key: format!("prompt-number-{}", i),
        };
        let idx = policy.select_with_context(&workers, &ctx);
        counts[idx] += 1;
    }

    // With 1000 different prefixes hashed to 10 workers, expect reasonable spread.
    let zeros = counts.iter().filter(|&&c| c == 0).count();
    assert!(zeros <= 1, "{} of 10 workers got zero traffic", zeros);
}

#[test]
fn test_prefix_affinity_single_worker() {
    let policy = PrefixAffinity::new(1);
    let workers = make_workers(1);
    let ctx = RequestContext {
        session_id: "u1".into(),
        prefix_key: "any-prompt".into(),
    };
    assert_eq!(policy.select_with_context(&workers, &ctx), 0);
}

#[test]
fn test_prefix_affinity_active_tracking() {
    let policy = PrefixAffinity::new(2);
    let workers = make_workers(2);

    policy.on_request_start(0);
    policy.on_request_start(0);

    // Without context, should pick JSQ (worker 1 has 0 active).
    let chosen = policy.select(&workers);
    assert_eq!(chosen, 1);

    policy.on_request_end(0);
    policy.on_request_end(0);
}
