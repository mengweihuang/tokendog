//! Random worker selection policy.

use rand::Rng;

use super::LoadBalancer;

/// A random-selection load balancer.
///
/// Picks a worker uniformly at random on each call. Stateless and contention-free.
pub struct Random;

impl LoadBalancer for Random {
    fn select(&self, workers: &[String]) -> usize {
        rand::thread_rng().gen_range(0..workers.len())
    }
}
