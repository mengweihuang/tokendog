# tokendog-router

LLM gateway for [vLLM](https://github.com/vllm-project/vllm) / [SGLang](https://github.com/sgl-project/sglang) local inference engines.

## Design Philosophy

### Minimal, transparent proxy

The gateway is deliberately thin — it does not parse, inspect, or modify request payloads.
Every HTTP request (method, headers, body, query parameters) is forwarded verbatim to the
selected worker. This ensures compatibility with any existing vLLM/SGLang client code
and avoids coupling the gateway to specific API versions.

### Streaming-first

LLM inference is fundamentally streaming — tokens arrive one at a time over Server-Sent
Events (SSE). The response path never buffers: worker chunks are forwarded to the client
as they arrive via `reqwest::bytes_stream()` → `axum::Body::from_stream()`. This design
keeps time-to-first-token minimal regardless of response size.

### Pluggable load balancing

Worker selection is abstracted behind the [`LoadBalancer`] trait so new strategies
(weighted, least-connections, consistent-hash, etc.) can be added without touching the
request-forwarding logic.

## Architecture

```
crates/router/src/
├── main.rs              # Entry point: config → state → server → graceful shutdown
├── lib.rs               # Module tree + build_router() wiring
├── config.rs            # CLI/env configuration (listen addr, worker_urls, timeout)
├── state.rs             # AppState: shared HTTP client + worker list + load balancer
├── proxy.rs             # Core reverse-proxy handler (the hot path)
├── health.rs            # GET /health endpoint
├── header.rs            # RFC 2616 hop-by-hop header filtering
├── error.rs             # ProxyError → HTTP status codes (no stack leaks)
└── policies/
    ├── mod.rs           # LoadBalancer trait
    └── round_robin.rs   # RoundRobin implementation (lock-free AtomicUsize)
```

### Request flow

```
Client                     tokendog-router                    Worker vLLM
  │                             │                                  │
  │  POST /v1/chat/completions   │                                  │
  │─────────────────────────────►│                                  │
  │                             │  next_worker() (round-robin)     │
  │                             │─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─►│
  │                             │                                  │
  │                             │  Forward request (verbatim)      │
  │                             │─────────────────────────────────►│
  │                             │                                  │
  │                             │  SSE: data: {"delta":{"content":  │
  │                             │◄─────────────────────────────────│
  │                             │                                  │
  │  Stream response chunks     │                                  │
  │◄─────────────────────────────│                                  │
```

## Configuration

All options can be set via CLI arguments or environment variables:

| Option | Env var | Default | Description |
|--------|---------|---------|-------------|
| `--host` | `HOST` | `0.0.0.0` | Gateway listen host |
| `--port` | `PORT` | `30000` | Gateway listen port |
| `--worker-urls` | `WORKER_URLS` | *(required)* | Comma-separated worker URLs |
| `--request-timeout-secs` | `REQUEST_TIMEOUT` | `300` | Worker response timeout |

```bash
# CLI args
cargo run -- --worker-urls http://192.168.1.10:8000,http://192.168.1.20:8000

# Environment variables
WORKER_URLS=http://192.168.1.10:8000,http://192.168.1.20:8000 cargo run
```

## Load Balancing Policies

The [`LoadBalancer`] trait allows switching strategies without modifying the proxy logic:

```rust
pub trait LoadBalancer: Send + Sync {
    fn select(&self, workers: &[String]) -> usize;
}
```

### Built-in policies

| Policy | File | Strategy |
|--------|------|----------|
| `RoundRobin` | `policies/round_robin.rs` | Lock-free `AtomicUsize` counter, `Ordering::Relaxed` |

### Adding a new policy

Create a new file in `policies/`:

```rust
// crates/router/src/policies/weighted.rs
pub struct Weighted {
    weights: Vec<usize>,
    counter: AtomicUsize,
}

impl LoadBalancer for Weighted {
    fn select(&self, workers: &[String]) -> usize {
        // Custom selection logic
    }
}
```

Then register it in `policies/mod.rs` and use it via `AppState::new(worker_urls, timeout, Weighted::new(...))`.

## SSE Streaming

The response path is fully streaming — critical for LLM inference where each token
arrives as a separate SSE data frame:

1. Worker responds with `Content-Type: text/event-stream`
2. `reqwest::Response::bytes_stream()` yields chunks as they arrive from the worker
3. `axum::Body::from_stream()` feeds each chunk directly into the HTTP response body
4. The client receives tokens incrementally with zero buffering

The request body is collected to `Bytes` (typically small JSON, acceptable buffer)
because `axum::body::Body` is `!Send` and cannot be passed directly to reqwest.

## Error Handling

- **502 Bad Gateway**: Worker URL parse failure, connection refused, or transport error
- **504 Gateway Timeout**: Worker does not respond within `request-timeout-secs`
- **500 Internal Server Error**: Response construction failure (should not occur in practice)
- **400 Bad Request**: Request body too large or unreadable

All errors are logged via `tracing` with internal details hidden from the response body.

## Dependencies

| Crate | Role |
|-------|------|
| [axum] | HTTP server framework (tokio/hyper based) |
| [reqwest] | HTTP client with connection pooling |
| [tokio] | Async runtime |
| [clap] | CLI argument parsing + env var support |
| [tower-http] | Tracing middleware |
| [tracing] / [tracing-subscriber] | Structured logging |
| [serde] / [serde_json] | JSON serialization (health endpoint) |
| [url] | URL parsing and validation |
| [http-body-util] | Request body collection |

[axum]: https://crates.io/crates/axum
[reqwest]: https://crates.io/crates/reqwest
[tokio]: https://crates.io/crates/tokio
[clap]: https://crates.io/crates/clap
[tower-http]: https://crates.io/crates/tower-http
[tracing]: https://crates.io/crates/tracing
[tracing-subscriber]: https://crates.io/crates/tracing-subscriber
[serde]: https://crates.io/crates/serde
[serde_json]: https://crates.io/crates/serde_json
[url]: https://crates.io/crates/url
[http-body-util]: https://crates.io/crates/http-body-util
