//! Finite state machine for request lifecycle management.

use crate::core::TokenContainer;
use crate::resource::allocator::{OwnedPages, ReqPoolIndex};

pub mod events;

/// All possible request lifecycle states.
#[derive(Debug)]
pub enum State {
    Bootstrapping,
    Submitted(Submitted),
    Prefetching(Prefetching),
    PrefetchDone(PrefetchDone),
    Aborting(Aborting),
    Prefilling(Prefilling),
    PrefillDone(PrefillDone),
    Decoding(Decoding),
    Draining(Draining),
    WritingBack(WritingBack),
    Retracting(Retracting),
    Retracted(Retracted),
    Finished,
}

// ---------------------------------------------------------------------------
// State structs
// ---------------------------------------------------------------------------

/// Submitted: holds only the token container and page size.
#[derive(Debug)]
pub struct Submitted {
    pub token_container: TokenContainer,
    pub page_size: i32,
}

/// Prefetching: waiting for async host-page prefetch.
#[derive(Debug)]
pub struct Prefetching {
    pub host_pages: OwnedPages,
}

/// PrefetchDone: prefetch complete, ready to schedule prefill.
#[derive(Debug)]
pub struct PrefetchDone {
    pub host_pages: OwnedPages,
}

/// Aborting: prefetch being aborted.
#[derive(Debug)]
pub struct Aborting {
    pub host_pages: OwnedPages,
}

/// Base state with common fields shared by forward states.
#[derive(Debug)]
pub struct BaseState {
    pub token_container: TokenContainer,
    pub page_size: i32,
}

/// Forward state with active device resource and allocators.
#[derive(Debug)]
pub struct ForwardState {
    pub base: BaseState,
    pub req_pool_index: ReqPoolIndex,
}

/// Prefilling: currently processing prefill tokens.
#[derive(Debug)]
pub struct Prefilling {
    pub base: ForwardState,
    pub window_begin: i32,
    pub window_size: i32,
    pub reserve_num_tokens: i32,
}

/// All prefill tokens have been scheduled.
#[derive(Debug)]
pub struct PrefillDone {
    pub base: ForwardState,
    pub window_begin: i32,
    pub window_size: i32,
    pub reserve_num_tokens: i32,
}

/// Auto-regressive decoding.
#[derive(Debug)]
pub struct Decoding {
    pub base: ForwardState,
    pub reserve_num_tokens: i32,
}

/// Generation finished, device-to-host writeback pending.
#[derive(Debug)]
pub struct Draining {
    pub pages_to_transfer: Vec<TransferPair>,
}

/// Writeback operation dispatched, awaiting completion.
#[derive(Debug)]
pub struct WritingBack {
    pub pages_to_transfer: Vec<TransferPair>,
    pub is_retract: bool,
}

/// Preempted, device-to-host writeback in flight.
#[derive(Debug)]
pub struct Retracting {
    pub base: ForwardState,
    pub pages_to_transfer: Vec<TransferPair>,
}

/// Preempted, pages on host, can be recovered via LoadBack.
#[derive(Debug)]
pub struct Retracted {
    pub token_container: TokenContainer,
    pub page_size: i32,
}

/// Transfer pair for device↔host page copies.
#[derive(Debug, Clone)]
pub struct TransferPair {
    pub device_page: i32,
    pub host_page: i32,
}

/// FSM error type.
#[derive(Debug, thiserror::Error)]
pub enum FsmError {
    #[error("invalid transition from {from} by {event}")]
    InvalidTransition { from: String, event: String },
}
