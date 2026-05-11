/// Hop-by-hop header filtering utilities.
///
/// RFC 2616 Section 13.5.1 defines headers that must not be forwarded
/// by intermediaries.
use axum::http::{HeaderMap, HeaderName, HeaderValue};

/// Headers that MUST NOT be forwarded from the backend response to the client.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// Check whether a header name is a hop-by-hop header that should not be forwarded.
pub fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP.contains(&name)
}

/// Return a filtered copy of backend response headers, excluding hop-by-hop headers.
pub fn filter_hop_by_hop(headers: &HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
    headers
        .iter()
        .filter(|(name, _)| !is_hop_by_hop(name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
