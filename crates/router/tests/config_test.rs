use clap::Parser;
use router::config::{Config, LogLevel};

#[test]
fn test_log_level_to_tracing_level() {
    assert_eq!(LogLevel::Error.to_tracing_level(), tracing::Level::ERROR);
    assert_eq!(LogLevel::Warn.to_tracing_level(), tracing::Level::WARN);
    assert_eq!(LogLevel::Info.to_tracing_level(), tracing::Level::INFO);
    assert_eq!(LogLevel::Debug.to_tracing_level(), tracing::Level::DEBUG);
}

#[test]
fn test_config_defaults() {
    let config = Config::try_parse_from(["router", "--worker-urls", "http://localhost:8000"])
        .expect("Config should parse with valid worker URL");
    assert_eq!(config.host, "0.0.0.0");
    assert_eq!(config.port, 30000);
    assert_eq!(config.worker_urls, vec!["http://localhost:8000"]);
    assert_eq!(config.request_timeout_secs, 300);
    assert!(config.log_file.is_none());
}

#[test]
fn test_config_custom_values() {
    let config = Config::try_parse_from([
        "router",
        "--host",
        "127.0.0.1",
        "--port",
        "8080",
        "--worker-urls",
        "http://worker1:8000",
        "http://worker2:8000",
        "--request-timeout-secs",
        "60",
        "--log-level",
        "debug",
    ])
    .expect("Config should parse with custom values");

    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 8080);
    assert_eq!(config.worker_urls.len(), 2);
    assert!(config
        .worker_urls
        .contains(&"http://worker1:8000".to_string()));
    assert!(config
        .worker_urls
        .contains(&"http://worker2:8000".to_string()));
    assert_eq!(config.request_timeout_secs, 60);
}

#[test]
fn test_config_single_worker() {
    let config = Config::try_parse_from(["router", "--worker-urls", "http://localhost:8000"])
        .expect("Config should parse with single worker");
    assert_eq!(config.worker_urls.len(), 1);
    assert_eq!(config.worker_urls[0], "http://localhost:8000");
}

#[test]
fn test_config_data_plane_api_keys_single() {
    let config = Config::try_parse_from([
        "router",
        "--worker-urls",
        "http://localhost:8000",
        "--data-plane-api-keys",
        "sk-test123",
    ])
    .expect("Config should parse with data plane API key");
    assert_eq!(config.data_plane_api_keys, vec!["sk-test123"]);
}

#[test]
fn test_config_data_plane_api_keys_multiple_flags() {
    let config = Config::try_parse_from([
        "router",
        "--worker-urls",
        "http://localhost:8000",
        "--data-plane-api-keys",
        "sk-key1",
        "--data-plane-api-keys",
        "sk-key2",
        "--data-plane-api-keys",
        "sk-key3",
    ])
    .expect("Config should parse with multiple --data-plane-api-keys flags");
    assert_eq!(
        config.data_plane_api_keys,
        vec!["sk-key1", "sk-key2", "sk-key3"]
    );
}

#[test]
fn test_config_data_plane_api_keys_empty_by_default() {
    let config = Config::try_parse_from(["router", "--worker-urls", "http://localhost:8000"])
        .expect("Config should parse without data plane API keys");
    assert!(config.data_plane_api_keys.is_empty());
}

#[test]
fn test_config_log_file_env() {
    let config = Config::try_parse_from([
        "router",
        "--worker-urls",
        "http://localhost:8000",
        "--log-file",
        "/var/log/router.json",
    ])
    .expect("Config should parse with log-file");
    assert_eq!(config.log_file, Some("/var/log/router.json".to_string()));
}

#[test]
fn test_config_multiple_workers() {
    let config = Config::try_parse_from([
        "router",
        "--worker-urls",
        "http://worker1:8000",
        "http://worker2:8000",
        "http://worker3:8000",
    ])
    .expect("Config should parse with multiple workers");
    assert_eq!(config.worker_urls.len(), 3);
    assert_eq!(config.worker_urls[0], "http://worker1:8000");
    assert_eq!(config.worker_urls[1], "http://worker2:8000");
    assert_eq!(config.worker_urls[2], "http://worker3:8000");
}
