---
name: policy-skill
description: Add a new load-balancing policy to the router crate and its Python bindings.
---

# Policy Skill

Add a new load-balancing policy (`LoadBalancer` trait implementation) to the
router, wiring it through both the Rust binary and Python bindings.

## Architecture

The `LoadBalancer` trait at `crates/router/src/policies/mod.rs`:

```rust
pub trait LoadBalancer: Send + Sync {
    fn select(&self, workers: &[String]) -> usize;
}
```

Return a `usize` index into the `workers` slice — callers pass it directly to
`workers[idx]`.

## Touch points (all required)

| # | File | Action |
|---|------|--------|
| 1 | `crates/router/src/policies/<name>.rs` | Create — implement `LoadBalancer` |
| 2 | `crates/router/src/policies/mod.rs` | Add `pub mod <name>;` |
| 3 | `crates/router/src/config/mod.rs` | Add variant to `Policy` enum |
| 4 | `crates/router/src/main.rs` | Add match arm in policy dispatch |
| 5 | `crates/router-python/src/lib.rs` | Add match arm in `serve()` |
| 6 | `crates/router-python/src/router/cli.py` | Add choice to `--policy` argparse |

## Steps

### 1. Create the policy file

`crates/router/src/policies/<name>.rs` — structure:

```rust
//! <one-line description>

use super::LoadBalancer;

/// <doc comment>
pub struct MyPolicy;

impl LoadBalancer for MyPolicy {
    fn select(&self, workers: &[String]) -> usize {
        // return workers.len() - 1  // example
        todo!()
    }
}
```

If the policy needs state, use `Send + Sync` types (e.g. `AtomicUsize`, `Mutex`,
`RwLock`).

### 2. Register in `mod.rs`

```rust
pub mod <name>;
// keep alphabetical
```

### 3. Add CLI variant

In `config/mod.rs`, add to `Policy` enum:

```rust
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum Policy {
    RoundRobin,
    Random,
    MyPolicy,         // <-- new
}
```

Then add a mapping entry in the `#[arg(long)]` `default_value` for `--policy`:

The `default_value = "round-robin"` stays on the field; the new variant uses
clap's automatic kebab-case conversion (e.g. `MyPolicy` → `--policy my-policy`).

### 4. Wire in `main.rs`

Add a match arm:

```rust
router::config::Policy::MyPolicy => Arc::new(AppState::new(
    config.worker_urls.clone(),
    config.request_timeout_secs,
    MyPolicy::new(),
)),
```

### 5. Wire in Python `lib.rs`

Add a match arm in `serve()`:

```rust
"my-policy" => Arc::new(AppState::new(
    worker_urls,
    timeout_secs,
    MyPolicy::new(),
)),
```

### 6. Add CLI choice in Python

In `cli.py`, add to `choices` list for `--policy`:

```python
choices=["round-robin", "random", "my-policy"],
```

### 7. Build

```bash
cd crates && cargo check
```

## Test

```bash
cd crates && cargo test
```
