use axum::http::StatusCode;
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use router::middleware::error::ProxyError;

#[test]
fn test_proxy_error_url_parse_returns_502() {
    let parse_err = "".parse::<url::Url>().unwrap_err();
    let err = ProxyError::UrlParse(parse_err);
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[test]
fn test_proxy_error_body_collect_returns_400() {
    let err = ProxyError::BodyCollect("request body too large".to_string());
    let response = err.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn test_proxy_error_into_response_content_type() {
    let err = ProxyError::BodyCollect("failed".to_string());
    let response = err.into_response();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("should have content-type header");
    assert_eq!(content_type, "text/plain; charset=utf-8");
}

#[tokio::test]
async fn test_proxy_error_body_collect_body_content() {
    let err = ProxyError::BodyCollect("custom error".to_string());
    let response = err.into_response();

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_text = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(body_text, "failed to read request body");
}

#[tokio::test]
async fn test_proxy_error_url_parse_body_content() {
    let parse_err = "".parse::<url::Url>().unwrap_err();
    let err = ProxyError::UrlParse(parse_err);
    let response = err.into_response();

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_text = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(body_text, "invalid worker URL");
}
