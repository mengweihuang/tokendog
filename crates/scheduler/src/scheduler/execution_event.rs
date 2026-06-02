//! Execution events: input to the scheduler's Advance method.

/// Events reported back to the scheduler after plan execution.
#[derive(Debug, Clone)]
pub enum ExecutionEvent {
    /// Extend token container with result tokens.
    ExtendResult {
        request_id: String,
        result_tokens: Vec<i32>,
    },
    /// Request finished generation.
    Finish {
        request_id: String,
    },
    /// Abort a request.
    Abort {
        request_id: String,
        reason: String,
    },
    /// Update reserve num tokens for next schedule event.
    UpdateReserveNumTokens {
        request_id: String,
        num_tokens: i32,
    },
    /// Prefetch completed.
    PrefetchDone {
        request_id: String,
    },
    /// WriteBack completed.
    WriteBackDone {
        request_id: String,
    },
    /// Bootstrapping completed (disaggregation).
    Bootstrapped {
        request_id: String,
    },
    /// Succeeded event (disaggregation).
    Succeeded {
        request_id: String,
    },
    /// Failed event (disaggregation).
    Failed {
        request_id: String,
        reason: String,
    },
    /// Remote prefill done.
    RemotePrefillDone {
        request_id: String,
    },
}
