//! Authentication middleware for data plane API keys.
//!
//! API keys are SHA-256 hashed at construction time and never stored
//! in plain text. Incoming Bearer tokens are verified using constant-time
//! comparison to prevent timing side-channel attacks.

use axum::{
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Pre-computed SHA-256 hashes of valid API keys.
///
/// Keys are hashed at construction time and the original plain-text
/// strings are discarded. This prevents accidental leakage of credentials
/// through memory dumps, logs, or debugging inspection.
#[derive(Clone)]
pub struct AuthConfig {
    api_key_hashes: Option<Vec<[u8; 32]>>,
}

impl AuthConfig {
    /// Create a new `AuthConfig` from a list of plain-text API keys.
    ///
    /// Each key is SHA-256 hashed immediately and the original strings
    /// are discarded. An empty list or `None` means authentication is
    /// disabled (all requests pass through).
    pub fn new(api_keys: Option<Vec<String>>) -> Self {
        Self {
            api_key_hashes: api_keys.map(|keys| {
                keys.into_iter()
                    .map(|k| Sha256::digest(k.as_bytes()).into())
                    .collect()
            }),
        }
    }
}

/// Axum middleware that validates `Authorization: Bearer <token>` against
/// the configured API key hashes.
///
/// # Behaviour
/// - The `/health` endpoint is always publicly accessible (no auth required).
/// - If no API keys are configured (hashes is `None` or empty), all requests
///   pass through — this allows the router to run without auth in trusted
///   environments.
/// - If keys are configured, the middleware extracts the Bearer token from
///   the `Authorization` header, SHA-256 hashes it, and compares against all
///   known hashes using constant-time comparison.
/// - On mismatch, returns `401 Unauthorized`.
pub async fn auth_middleware(
    State(config): State<AuthConfig>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // Health checks are always publicly accessible.
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }

    if let Some(expected_hashes) = &config.api_key_hashes {
        if !expected_hashes.is_empty() {
            let token = request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.strip_prefix("Bearer "));

            let authorized = token.is_some_and(|t| {
                let token_hash = Sha256::digest(t.as_bytes());
                expected_hashes.iter().any(|expected_hash| {
                    token_hash.as_slice().ct_eq(expected_hash).unwrap_u8() == 1
                })
            });

            if !authorized {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            }
        }
    }

    next.run(request).await
}
