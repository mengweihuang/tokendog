//! Execution plan: the output of the scheduler's planning phase.

use super::operations::{
    FlatForwardOperation, LoadBackOperation, PrefetchOperation, WriteBackOperation,
};

/// The result of a scheduler planning tick.
#[derive(Debug, Clone, Default)]
pub struct ExecutionPlan {
    /// Forward operations (prefill + decode) for the inference engine.
    pub forward: Vec<FlatForwardOperation>,
    /// Cache operations (write-back, load-back, prefetch).
    pub write_backs: Vec<WriteBackOperation>,
    pub load_backs: Vec<LoadBackOperation>,
    pub prefetches: Vec<PrefetchOperation>,
}

impl ExecutionPlan {
    pub fn is_empty(&self) -> bool {
        self.forward.iter().all(|f| f.is_empty())
            && self.write_backs.is_empty()
            && self.load_backs.is_empty()
            && self.prefetches.is_empty()
    }
}
