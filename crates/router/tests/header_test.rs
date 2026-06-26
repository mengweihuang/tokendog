use axum::http::{HeaderMap, HeaderValue};
use router::middleware::header::{filter_hop_by_hop, is_hop_by_hop};

#[test]
fn test_is_hop_by_hop_known_headers() {
    assert!(is_hop_by_hop("connection"));
    assert!(is_hop_by_hop("keep-alive"));
    assert!(is_hop_by_hop("proxy-authenticate"));
    assert!(is_hop_by_hop("proxy-authorization"));
    assert!(is_hop_by_hop("te"));
    assert!(is_hop_by_hop("trailers"));
    assert!(is_hop_by_hop("transfer-encoding"));
    assert!(is_hop_by_hop("upgrade"));
}

#[test]
fn test_is_hop_by_hop_normal_headers() {
    assert!(!is_hop_by_hop("content-type"));
    assert!(!is_hop_by_hop("content-length"));
    assert!(!is_hop_by_hop("authorization"));
    assert!(!is_hop_by_hop("x-custom-header"));
    assert!(!is_hop_by_hop("host"));
}

#[test]
fn test_filter_hop_by_hop_removes_all_hop_by_hop() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("connection", HeaderValue::from_static("close"));
    headers.insert("transfer-encoding", HeaderValue::from_static("chunked"));
    headers.insert("x-request-id", HeaderValue::from_static("abc-123"));

    let filtered = filter_hop_by_hop(&headers);

    let names: Vec<&str> = filtered.iter().map(|(n, _)| n.as_str()).collect();

    assert_eq!(filtered.len(), 2);
    assert!(names.contains(&"content-type"));
    assert!(names.contains(&"x-request-id"));
    assert!(!names.contains(&"connection"));
    assert!(!names.contains(&"transfer-encoding"));
}

#[test]
fn test_filter_hop_by_hop_empty_headers() {
    let headers = HeaderMap::new();
    let filtered = filter_hop_by_hop(&headers);
    assert!(filtered.is_empty());
}

#[test]
fn test_filter_hop_by_hop_keeps_all_allowed_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", HeaderValue::from_static("Bearer token"));
    headers.insert("content-type", HeaderValue::from_static("text/plain"));

    let filtered = filter_hop_by_hop(&headers);
    assert_eq!(filtered.len(), 2);
}
