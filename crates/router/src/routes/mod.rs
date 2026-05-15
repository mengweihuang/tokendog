//! Prefill-Decode (PD) separation route handlers.
//!
//! Implements two-stage request processing for vLLM disaggregated inference:
//! prefill on a dedicated prefill worker, then decode on a dedicated decode worker.

pub mod logprobs_merge;
pub mod pd_handler;
pub mod prefill;

pub use pd_handler::pd_proxy_handler;
