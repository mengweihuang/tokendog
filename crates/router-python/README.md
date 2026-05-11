# router — Python Bindings for router

Python wheel for the router LLM gateway, built with [PyO3](https://pyo3.rs/) and [maturin](https://www.maturin.rs/).

## Installation

### From wheel

```bash
cd crates/router-python
maturin build --release
pip install ../target/wheels/router-*.whl
```

### Development install

```bash
cd crates/router-python
maturin develop
```

Requires Python ≥ 3.8 and a Rust toolchain.

## API

### `Router`

```python
from router import Router

gateway = Router(
    worker_urls: list[str],           # required — backend worker URLs
    host: str = "0.0.0.0",           # bind address
    port: int = 30000,               # bind port
    request_timeout_secs: int = 300,  # upstream timeout (seconds)
    log_level: str = "info",         # error | warn | info | debug
)
```

#### Properties

| Property | Type | Description |
|----------|------|-------------|
| `worker_urls` | `list[str]` | Configured backend worker URLs |
| `host` | `str` | Bind address |
| `port` | `int` | Bind port |
| `request_timeout_secs` | `int` | Upstream request timeout in seconds |
| `log_level` | `str` | Log level filter string |

#### `serve()`

Start the gateway server. Blocks until Ctrl+C or SIGTERM.

```python
gateway.serve()
```

## Usage

### Basic

```python
from router import Router

gateway = Router(
    worker_urls=[
        "http://192.168.1.10:8000",
        "http://192.168.1.20:8000",
    ],
    port=30000,
    log_level="info",
)

gateway.serve()
```

### With health check

After starting the gateway in one process, query it from another:

```python
import requests

resp = requests.get("http://localhost:30000/health")
print(resp.json())
# {"status": "ok", "worker_urls": ["http://192.168.1.10:8000", ...]}
```

### Multiple workers with round-robin

```python
from router import Router

workers = [f"http://10.0.0.{i}:8000" for i in range(1, 5)]

gateway = Router(
    worker_urls=workers,
    port=8080,
    request_timeout_secs=600,
    log_level="debug",
)

gateway.serve()
# Requests are distributed across 4 workers via lock-free round-robin
```

## How It Works

The `Router` class wraps the [router](../router/) library crate via PyO3:

1. **`__init__`** stores configuration in Rust struct fields
2. **`serve()`** releases the Python GIL (`py.allow_threads`), creates a Tokio runtime, builds the axum router with `AppState` + `RoundRobin` policy, binds a `TcpListener`, and serves until shutdown signal (Ctrl+C / SIGTERM)

All request forwarding, load balancing, header filtering, and SSE streaming run entirely in Rust — the Python side is a thin configuration and lifecycle layer.

## Related

- [router](../router/) — Rust crate
- [Project README](../../README.md) — full project overview
