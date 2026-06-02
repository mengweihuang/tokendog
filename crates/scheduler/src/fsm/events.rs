//! FSM events: each event struct applies a state transition.

use super::*;

// ---------------------------------------------------------------------------
// Forward events
// ---------------------------------------------------------------------------

/// Schedule the first chunk of prefill tokens.
pub struct SchedulePrefillFirstChunkEvent {
    pub prefill_input_ids: Vec<i32>,
    pub shifted_input_ids: Vec<i32>,
    pub occupied_pages: Vec<i32>,
    pub extend_prefix_len: i32,
    pub page_size: i32,
    pub reserve_num_tokens: i32,
}

/// Schedule a subsequent prefill chunk.
pub struct SchedulePrefillEvent {
    pub prefill_input_ids: Vec<i32>,
    pub shifted_input_ids: Vec<i32>,
    pub occupied_pages: Vec<i32>,
    pub extend_prefix_len: i32,
    pub page_size: i32,
    pub reserve_num_tokens: i32,
}

/// Schedule a decode step.
pub struct ScheduleDecodeEvent {
    pub decode_input_id: i32,
    pub occupied_pages: Vec<i32>,
    pub page_size: i32,
    pub hist_token_len: i32,
    pub reserve_num_tokens: i32,
}

/// Schedule decode from retracted state.
pub struct ScheduleDecodeFromRetractedEvent {
    pub decode_input_id: i32,
    pub page_size: i32,
    pub reserve_num_tokens: i32,
}

/// Schedule a retraction (preemption).
pub struct ScheduleRetractEvent {
    pub pages_to_transfer: Vec<TransferPair>,
}

/// Finish request (generation complete).
pub struct FinishEvent;

/// Abort request.
pub struct AbortEvent {
    pub reason: String,
}

/// Commit draining state to WritingBack.
pub struct CommitDrainingEvent;

/// Extend the token container with result tokens.
pub struct ExtendResultEvent {
    pub result_tokens: Vec<i32>,
}

/// Update reserve num tokens for next schedule event.
pub struct UpdateReserveNumTokensEvent {
    pub num_tokens: i32,
}

// ---------------------------------------------------------------------------
// Cache events
// ---------------------------------------------------------------------------

/// Schedule async host-page prefetch.
pub struct SchedulePrefetchEvent {
    pub host_pages: OwnedPages,
}

/// Prefetch completed.
pub struct PrefetchDoneEvent;

/// WriteBack completed.
pub struct WriteBackDoneEvent;

// ---------------------------------------------------------------------------
// PD events (disaggregation)
// ---------------------------------------------------------------------------

/// Bootstrapping completed.
pub struct BootstrappedEvent;

/// Prefill succeeded (disaggregated).
pub struct SucceededEvent;

/// Prefill failed (disaggregated).
pub struct FailedEvent {
    pub reason: String,
}

/// Remote prefill done.
pub struct RemotePrefillDoneEvent;
