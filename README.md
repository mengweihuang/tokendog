# TokenDog

LLM gateway for [vLLM](https://github.com/vllm-project/vllm) / [SGLang](https://github.com/sgl-project/sglang) local inference engines — thin reverse proxy with load balancing and streaming support.

## Packages

| Package | Language | Description |
|---------|----------|-------------|
| [router](crates/router/) | Rust | HTTP gateway binary + library crate |
| [router python](crates/router-python/) | Python | PyO3 bindings, installable as a wheel |

## Quick Start

### Rust binary

```bash
cd crates
cargo run -- \
    --worker-urls http://192.168.1.10:8000 http://192.168.1.20:8000 \
    --port 30000 \
    --log-level info
```

### Python wheel

```bash
cd crates/router-python
maturin build --release
pip install ../target/wheels/router-*.whl
```

Use from the command line:

```bash
router --port 30000 --worker-urls http://192.168.1.10:8000 http://192.168.1.20:8000
```

Or as a library in Python:

```python
from router import Router

gateway = Router(
    worker_urls=["http://192.168.1.10:8000", "http://192.168.1.20:8000"],
    port=30000,
)
gateway.serve()  # blocks until Ctrl+C
```

See [examples/](examples/) for more.

## Architecture

```
Client                  tokendog                   Backend vLLM/SGLang
  │                        │                              │
  │  POST /v1/chat/...     │                              │
  │───────────────────────►│  next_worker() (round-robin) │
  │                        │─────────────────────────────►│
  │                        │                              │
  │                        │  SSE token stream            │
  │  Streamed response     │◄─────────────────────────────│
  │◄───────────────────────│                              │
```

- **Transparent**: requests forwarded verbatim — no API coupling
- **Streaming-first**: SSE frames forwarded without buffering (`bytes_stream → from_stream`)
- **Pluggable LB**: `LoadBalancer` trait — 7 built-in policies including cache-aware routing (session affinity, prefix affinity, load-cache-aware scoring)

## Configuration

All options via CLI args or env vars:

| Option | Env | Default | Description |
|--------|-----|---------|-------------|
| `--host` | `HOST` | `0.0.0.0` | Bind address |
| `--port` | `PORT` | `30000` | Bind port |
| `--worker-urls` | `WORKER_URLS` | *(required)* | Backend URLs (space-separated) |
| `--request-timeout-secs` | `REQUEST_TIMEOUT` | `300` | Worker timeout (seconds) |
| `--log-level` | `LOG_LEVEL` | `info` | Log filter: error, warn, info, debug |
| `--policy` | `POLICY` | `least-loaded` | Load-balancing policy (see [router README](crates/router/README.md)) |

## Development

```bash
# Rust workspace
cd crates
cargo build
cargo test
cargo clippy --all-targets

# Python bindings
cd crates/router-python
maturin develop
python -c "from router import Router; print(Router(worker_urls=['http://localhost:8000']))"
```

## License

Apache-2.0
