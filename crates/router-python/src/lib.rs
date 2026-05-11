//! Python bindings for router — LLM gateway for vLLM/SGLang inference engines.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use pyo3::prelude::*;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use router::build_router;
use router::policies::round_robin::RoundRobin;
use router::shutdown_signal;
use router::state::AppState;

/// Python-facing gateway router.
///
/// Wraps the router HTTP proxy with load-balanced worker selection.
///
/// Args:
///     worker_urls: List of backend worker URLs (vLLM/SGLang endpoints).
///     host: Bind address for the HTTP listener.
///     port: Bind port for the HTTP listener.
///     request_timeout_secs: Timeout in seconds for upstream requests.
///     log_level: Log level — one of "error", "warn", "info", "debug".
#[pyclass]
struct Router {
    #[pyo3(get)]
    worker_urls: Vec<String>,
    #[pyo3(get)]
    host: String,
    #[pyo3(get)]
    port: u16,
    #[pyo3(get)]
    request_timeout_secs: u64,
    #[pyo3(get)]
    log_level: String,
}

#[pymethods]
impl Router {
    #[new]
    #[pyo3(signature = (worker_urls, host="0.0.0.0", port=30000, request_timeout_secs=300, log_level="info"))]
    fn new(
        worker_urls: Vec<String>,
        host: &str,
        port: u16,
        request_timeout_secs: u64,
        log_level: &str,
    ) -> Self {
        Router {
            worker_urls,
            host: host.to_string(),
            port,
            request_timeout_secs,
            log_level: log_level.to_string(),
        }
    }

    /// Start the gateway server (blocking call, runs until shutdown signal).
    ///
    /// Initializes tracing, builds the axum router, binds the listener, and
    /// serves requests. Blocks the calling thread until Ctrl+C or SIGTERM.
    fn serve(&self, py: Python<'_>) -> PyResult<()> {
        let level = match self.log_level.to_lowercase().as_str() {
            "error" => "error",
            "warn" => "warn",
            "info" => "info",
            "debug" => "debug",
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Invalid log_level '{}', expected one of: error, warn, info, debug",
                    self.log_level
                )))
            }
        };

        let ip: IpAddr = self.host.parse().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Invalid host '{}': {}",
                self.host, e
            ))
        })?;
        let addr = SocketAddr::new(ip, self.port);

        let worker_urls = self.worker_urls.clone();
        let timeout_secs = self.request_timeout_secs;

        // Release the GIL before blocking on the async server loop.
        let result: Result<(), String> = py.allow_threads(move || {
            match tracing_subscriber::fmt()
                .with_env_filter(EnvFilter::new(level))
                .try_init()
            {
                Ok(()) => {}
                Err(_) => {
                    eprintln!(
                        "[router] tracing subscriber already set; logs may go to a different subscriber"
                    );
                }
            }

            tracing::info!(
                host = %ip,
                port = addr.port(),
                worker_urls = ?worker_urls,
                "Starting router (Python bindings)",
            );

            let state = Arc::new(AppState::new(
                worker_urls,
                timeout_secs,
                RoundRobin::new(),
            ));

            let app = build_router(state);

            let rt = tokio::runtime::Runtime::new().map_err(|e| {
                format!("Failed to create Tokio runtime: {}", e)
            })?;

            rt.block_on(async move {
                tracing::info!("Listening on {}", addr);

                let listener = TcpListener::bind(addr).await.map_err(|e| {
                    format!("Failed to bind to {}: {}", addr, e)
                })?;

                axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown_signal())
                    .await
                    .map_err(|e| format!("Server error: {}", e))
            })
        });

        result.map_err(PyErr::new::<pyo3::exceptions::PyRuntimeError, String>)
    }
}

/// Python module: ``router._core`` — compiled Rust extension for the router gateway.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Router>()?;
    Ok(())
}
