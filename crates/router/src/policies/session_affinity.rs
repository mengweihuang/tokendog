use crate::policies::{stable_hash, LoadBalancer, RequestContext};

/// Routes requests from the same user/session deterministically to the same
/// worker, maximising KV-cache reuse for multi-turn conversations.
///
/// Stateless: uses a [`stable_hash`] of the `session_id` so the mapping
/// survives router restarts.
pub struct SessionAffinity;

impl LoadBalancer for SessionAffinity {
    fn select(&self, workers: &[String]) -> usize {
        if workers.len() == 1 {
            return 0;
        }
        // Without context, degenerate to random distribution.
        rand::random::<usize>() % workers.len()
    }

    fn select_with_context(&self, workers: &[String], ctx: &RequestContext) -> usize {
        let hash = stable_hash(&ctx.session_id);
        (hash as usize) % workers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_session_maps_to_same_worker() {
        let policy = SessionAffinity;
        let workers: Vec<String> = (0..5)
            .map(|i| format!("http://worker-{}:8000", i))
            .collect();

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
    fn different_sessions_distribute() {
        let policy = SessionAffinity;
        let workers: Vec<String> = (0..100).map(|i| format!("http://w{}", i)).collect();

        let mut counts = vec![0usize; workers.len()];
        for i in 0..1000 {
            let ctx = RequestContext {
                session_id: format!("user-{}", i),
                prefix_key: String::new(),
            };
            let idx = policy.select_with_context(&workers, &ctx);
            counts[idx] += 1;
        }

        // Every worker should get at least some traffic with 1000 users / 100 workers.
        let zeros = counts.iter().filter(|&&c| c == 0).count();
        assert!(zeros < 10, "{} workers got zero traffic", zeros);
    }
}
