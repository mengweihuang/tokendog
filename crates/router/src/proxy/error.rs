/// Unified error types for the proxy gateway.
///
/// Each variant maps to an appropriate HTTP status code without leaking internal details.
use axum::{
    body::Body,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use thiserror::Error;

/// Errors that can occur during request proxying.
#[derive(Debug, Error)]
pub enum ProxyError {
    /// The backend URL could not be parsed.
    #[error("Invalid backend URL: {0}")]
    UrlParse(#[from] url::ParseError),

    /// The request to the backend failed (timeout, connection refused, etc.).
    #[error("Backend request failed: {0}")]
    BackendRequest(#[from] reqwest::Error),

    /// Failed to collect the incoming request body.
    #[error("Failed to read request body: {0}")]
    BodyCollect(String),

    /// Failed to construct the outgoing HTTP response.
    #[error("Failed to build response: {0}")]
    ResponseBuild(#[from] axum::http::Error),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response<Body> {
        let (status, message) = match &self {
            Self::UrlParse(e) => {
                tracing::warn!("URL parse error: {e}");
                (StatusCode::BAD_GATEWAY, "invalid backend URL".to_string())
            }
            Self::BackendRequest(e) => {
                if e.is_timeout() {
                    tracing::warn!("Backend timeout: {e}");
                    (StatusCode::GATEWAY_TIMEOUT, "backend timeout".to_string())
                } else if e.is_connect() {
                    tracing::error!("Backend connection failed: {e}");
                    (
                        StatusCode::BAD_GATEWAY,
                        "backend connection failed".to_string(),
                    )
                } else {
                    tracing::error!("Backend request error: {e}");
                    (StatusCode::BAD_GATEWAY, "backend error".to_string())
                }
            }
            Self::BodyCollect(e) => {
                tracing::error!("Body collection error: {e}");
                (
                    StatusCode::BAD_REQUEST,
                    "failed to read request body".to_string(),
                )
            }
            Self::ResponseBuild(e) => {
                tracing::error!("Response build error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };

        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(message))
            .expect("valid status and header should always produce a Response")
    }
}
