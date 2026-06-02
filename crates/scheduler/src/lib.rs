//! scheduler — LLM inference request scheduler with KV prefix caching.
//!
//! Pure computation library (no I/O, no networking). Provides the core
//! scheduling loop, KV prefix cache, radix tree, and finite-state machine
//! for managing LLM inference request lifecycles.

pub mod core;
pub mod resource;
pub mod fsm;
pub mod scheduler;
