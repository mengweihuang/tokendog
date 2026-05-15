# TokenDog Router

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
├── main.rs              # Binary entry point: config → state → server → shutdown
├── lib.rs               # Module tree, build_router(), shutdown_signal()
├── config/
│   └── mod.rs           # CLI/env configuration (Config struct + Policy enum + LogLevel enum)
├── state/
│   └── mod.rs           # AppState: shared HTTP client + worker list + load balancer
├── proxy/
│   ├── mod.rs           # Core reverse-proxy handler (the hot path)
│   ├── error.rs         # ProxyError → HTTP status codes (no stack leaks)
│   └── header.rs        # RFC 2616 hop-by-hop header filtering
├── health/
│   └── mod.rs           # GET /health endpoint
├── routes/
│   ├── mod.rs           # PD route module declarations
│   ├── pd_handler.rs    # PD proxy handler (two-stage / dual dispatch)
│   ├── prefill.rs       # Prefill request construction + KV/boot params
│   └── logprobs_merge.rs # Prompt logprobs merge for PD mode
└── policies/
    ├── mod.rs           # LoadBalancer trait + RequestContext
    ├── least_loaded.rs  # Least-loaded (min in-flight requests)
    ├── power_of_two.rs  # Power of two choices
    ├── random.rs        # Uniform random
    ├── round_robin.rs   # Round-robin (lock-free AtomicUsize)
    ├── session_affinity.rs    # Deterministic hash on user/session_id
    ├── prefix_affinity.rs     # Hash on first-message prefix with queue-threshold fallback
    └── load_cache_aware.rs    # CacheDirectory + alpha/beta scoring
```

### Request flow

```
Client                         router                    Worker vLLM
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

## Quick Start

```bash
cargo run -- \
    --worker-urls http://192.168.1.10:8000 http://192.168.1.20:8000 \
    --port 30000 \
    --log-level info
```

Verify:

```bash
curl http://localhost:30000/health
# {"status":"ok","worker_urls":["http://192.168.1.10:8000","http://192.168.1.20:8000"]}
```

## Library API

The crate exposes a public library API so it can be embedded in other Rust projects
or used for Python bindings:

```rust
use std::sync::Arc;
use router::{
    build_router,
    policies::round_robin::RoundRobin,
    shutdown_signal,
    state::AppState,
};

let state = Arc::new(AppState::new(
    vec!["http://localhost:8000".into()],
    300,
    RoundRobin::new(),
));
let app = build_router(state);

let listener = tokio::net::TcpListener::bind("0.0.0.0:30000").await?;
axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await?;
```

### PD mode (library API)

```rust
use router::{
    build_router,
    config::PdMode,
    policies::round_robin::RoundRobin,
    state::AppState,
};

// vLLM PD: sequential two-stage
let state = AppState::new_pd(
    PdMode::Vllm,
    vec!["http://prefill1:8000".into(), "http://prefill2:8000".into()],
    vec!["http://decode1:8000".into(), "http://decode2:8000".into()],
    300,
    Box::new(RoundRobin::new()),
    Box::new(RoundRobin::new()),
);

// SGLang PD: concurrent dual dispatch
let state = AppState::new_pd(
    PdMode::Sglang,
    vec!["http://prefill1:8000".into()],
    vec!["http://decode1:8000".into()],
    300,
    Box::new(RoundRobin::new()),
    Box::new(RoundRobin::new()),
);
```

## Configuration

All options can be set via CLI arguments or environment variables:

| Option | Env var | Default | Description |
|--------|---------|---------|-------------|
| `--host` | `HOST` | `0.0.0.0` | Gateway listen host |
| `--port` | `PORT` | `30000` | Gateway listen port |
| `--worker-urls` | `WORKER_URLS` | *(required)* | Space-separated worker URLs |
| `--request-timeout-secs` | `REQUEST_TIMEOUT` | `300` | Worker response timeout |
| `--policy` | `POLICY` | `least-loaded` | Load-balancing policy (see below) |
| `--log-level` | `LOG_LEVEL` | `info` | Log filter: error, warn, info, debug |
| `--pd-mode` | `PD_MODE` | *(none)* | PD separation: `vllm` (sequential) or `sglang` (concurrent) |
| `--prefill-urls` | — | *(none)* | Prefill worker URLs for PD mode |
| `--decode-urls` | — | *(none)* | Decode worker URLs for PD mode |

```bash
# CLI args
cargo run -- --worker-urls http://192.168.1.10:8000 http://192.168.1.20:8000

# Environment variables
WORKER_URLS="http://192.168.1.10:8000 http://192.168.1.20:8000" cargo run
```

## Load Balancing Policies

The [`LoadBalancer`] trait allows switching strategies without modifying the proxy logic:

```rust
pub struct RequestContext {
    pub session_id: String,   // from "user" or "session_id" field
    pub prefix_key: String,   // first 200 chars of first message content
}

pub trait LoadBalancer: Send + Sync {
    fn select(&self, workers: &[String]) -> usize;

    // Override this for cache-aware routing; default calls select().
    fn select_with_context(&self, workers: &[String], ctx: &RequestContext) -> usize {
        self.select(workers)
    }

    fn on_request_start(&self, _worker_idx: usize) {}
    fn on_request_end(&self, _worker_idx: usize) {}

    // Override to update cache-affinity state after a completed request.
    fn record(&self, _ctx: &RequestContext, _worker_idx: usize) {}
}
```

### Built-in policies

| Policy | File | Strategy |
|--------|------|----------|
| `LeastLoaded` | `policies/least_loaded.rs` | Full-scan min in-flight requests |
| `PowerOfTwo` | `policies/power_of_two.rs` | Two random choices, pick the less busy |
| `Random` | `policies/random.rs` | Uniform random |
| `RoundRobin` | `policies/round_robin.rs` | Lock-free `AtomicUsize` counter |
| `SessionAffinity` | `policies/session_affinity.rs` | Deterministic hash on `user`/`session_id` |
| `PrefixAffinity` | `policies/prefix_affinity.rs` | Hash on first-message prefix, JSQ fallback when overloaded |
| `LoadCacheAware` | `policies/load_cache_aware.rs` | `alpha * cache_affinity - beta * load` scoring |

### Adding a new policy

Create a new file in `policies/`, declare it in `policies/mod.rs`, and add a variant to the `Policy` enum in `config/mod.rs`:

```rust
// crates/router/src/policies/weighted.rs
use std::sync::atomic::AtomicUsize;
use crate::policies::{LoadBalancer, RequestContext};

pub struct Weighted {
    weights: Vec<usize>,
    counter: AtomicUsize,
}

impl LoadBalancer for Weighted {
    fn select(&self, workers: &[String]) -> usize {
        // Load-only: delegate to select() default.
        // Cache-aware: override select_with_context() to inspect RequestContext.
    }
}
```

Then register it in `main.rs`, `router-python/src/lib.rs`, and `router-python/src/router/cli.py`.

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

| Status | Cause |
|--------|-------|
| **502 Bad Gateway** | Worker URL parse failure, connection refused, or transport error |
| **504 Gateway Timeout** | Worker does not respond within `request-timeout-secs` |
| **500 Internal Server Error** | Response construction failure |
| **400 Bad Request** | Request body too large (>16 MB) or unreadable |

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
