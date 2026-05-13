use router::policies::load_cache_aware::LoadCacheAware;
use router::policies::{LoadBalancer, RequestContext};

fn make_workers(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("http://worker-{}:8000", i))
        .collect()
}

#[test]
fn test_load_cache_aware_affinity_grows_after_record() {
    let policy = LoadCacheAware::with_params(3, 1.0, 0.0, 0.0);
    let workers = make_workers(3);
    let ctx = RequestContext {
        session_id: "u1".into(),
        prefix_key: "sys-a".into(),
    };

    let first = policy.select_with_context(&workers, &ctx);
    policy.record(&ctx, first);

    let second = policy.select_with_context(&workers, &ctx);
    assert_eq!(second, first);
}

#[test]
fn test_load_cache_aware_load_avoids_overloaded_worker() {
    let policy = LoadCacheAware::with_params(3, 1.0, 10.0, 0.0);
    let workers = make_workers(3);
    let ctx = RequestContext {
        session_id: "u1".into(),
        prefix_key: "sys-a".into(),
    };

    // Record worker 0 as having the cache.
    policy.record(&ctx, 0);

    // Saturate worker 0.
    for _ in 0..100 {
        policy.on_request_start(0);
    }

    // Should avoid worker 0 despite high affinity.
    let chosen = policy.select_with_context(&workers, &ctx);
    assert_ne!(chosen, 0);

    for _ in 0..100 {
        policy.on_request_end(0);
    }
}

#[test]
fn test_load_cache_aware_prefix_beats_session() {
    let policy = LoadCacheAware::with_params(3, 1.0, 0.0, 0.0);
    let workers = make_workers(3);

    // Record same session to worker 0.
    let ctx0 = RequestContext {
        session_id: "shared-user".into(),
        prefix_key: "prefix-a".into(),
    };
    policy.record(&ctx0, 0);

    // Different prefix, same session — initially no affinity for prefix-a on w0.
    let ctx1 = RequestContext {
        session_id: "shared-user".into(),
        prefix_key: "prefix-b".into(),
    };
    policy.record(&ctx1, 1);

    // Now prefix-a should still map to 0 (prefix match = 1.0 beats session = 0.5)
    let chosen = policy.select_with_context(&workers, &ctx0);
    assert_eq!(chosen, 0);
}

#[test]
fn test_load_cache_aware_single_worker() {
    let policy = LoadCacheAware::new(1);
    let workers = make_workers(1);
    let ctx = RequestContext {
        session_id: "u1".into(),
        prefix_key: "any".into(),
    };
    assert_eq!(policy.select_with_context(&workers, &ctx), 0);
}

#[test]
fn test_load_cache_aware_select_without_context() {
    let policy = LoadCacheAware::new(3);
    let workers = make_workers(3);
    // Falls back to JSQ.
    let idx = policy.select(&workers);
    assert!(idx < workers.len());
}

#[test]
fn test_load_cache_aware_staleness() {
    // With staleness = 0.0 seconds, entries never expire.
    let policy = LoadCacheAware::with_params(3, 1.0, 0.0, 0.0);
    let workers = make_workers(3);
    let ctx = RequestContext {
        session_id: "u1".into(),
        prefix_key: "p1".into(),
    };

    policy.record(&ctx, 0);

    // Wait a tiny bit (but staleness is 0 = never stale).
    std::thread::sleep(std::time::Duration::from_millis(1));

    let chosen = policy.select_with_context(&workers, &ctx);
    assert_eq!(chosen, 0);
}
