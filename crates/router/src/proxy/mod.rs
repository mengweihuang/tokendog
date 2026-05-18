//! Core reverse-proxy handler, error types, header filtering,
//! and request-context extraction.

pub mod context;
pub mod error;
pub mod handler;
pub mod header;

pub use handler::proxy_handler;
