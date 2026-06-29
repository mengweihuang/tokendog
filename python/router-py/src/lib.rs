//! Python bindings for router — LLM gateway for vLLM/SGLang inference engines.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use pyo3::prelude::*;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use std::collections::HashMap;

use router::config::auth::AuthConfig;
use router::config::{self, PdMode};
use router::policies::{
    least_loaded::LeastLoaded, load_cache_aware::LoadCacheAware, power_of_two::PowerOfTwo,
    prefix_affinity::PrefixAffinity, random::Random, round_robin::RoundRobin,
    session_affinity::SessionAffinity, LoadBalancer,
};
use router::server::{self, AppState};
use router::service_discovery::{self, ServiceDiscoveryConfig, SharedWorkerPool};
use router::worker::{self, Worker};

/// Validates a policy string and returns it normalized.
fn validate_policy(policy: &str) -> PyResult<String> {
    match policy.to_lowercase().as_str() {
        "least-loaded" | "least_loaded" => Ok("least-loaded".to_string()),
        "power-of-two" | "power_of_two" => Ok("power-of-two".to_string()),
        "random" => Ok("random".to_string()),
        "round-robin" | "round_robin" => Ok("round-robin".to_string()),
        "session-affinity" | "session_affinity" => Ok("session-affinity".to_string()),
        "prefix-affinity" | "prefix_affinity" => Ok("prefix-affinity".to_string()),
        "load-cache-aware" | "load_cache_aware" => Ok("load-cache-aware".to_string()),
        _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "Invalid policy '{}', expected one of: least-loaded, power-of-two, random, round-robin, session-affinity, prefix-affinity, load-cache-aware",
            policy
        ))),
    }
}

/// Construct a `Box<dyn LoadBalancer>` from a policy string and worker count.
fn make_policy(policy: &str, n: usize) -> Box<dyn LoadBalancer> {
    match policy {
        "least-loaded" => Box::new(LeastLoaded::new(n)),
        "power-of-two" => Box::new(PowerOfTwo::new(n)),
        "random" => Box::new(Random),
        "session-affinity" => Box::new(SessionAffinity),
        "prefix-affinity" => Box::new(PrefixAffinity::new(n)),
        "load-cache-aware" => Box::new(LoadCacheAware::new(n)),
        _ => Box::new(RoundRobin::new()),
    }
}

/// Python-facing gateway router.
///
/// Wraps the router HTTP proxy with load-balanced worker selection
/// and optional Bearer-token authentication.
///
/// Args:
///     worker_urls: List of backend worker URLs (vLLM/SGLang endpoints).
///     host: Bind address for the HTTP listener.
///     port: Bind port for the HTTP listener.
///     request_timeout_secs: Timeout in seconds for upstream requests.
///     log_level: Log level — one of "error", "warn", "info", "debug".
///     policy: Load-balancing policy — "least-loaded" (default), "power-of-two",
///         "random", "round-robin", "session-affinity", "prefix-affinity",
///         or "load-cache-aware".
///     pd_mode: Prefill-Decode separation mode — None (default), "vllm", or "sglang".
///     prefill_urls: Prefill worker URLs for PD mode.
///     decode_urls: Decode worker URLs for PD mode.
///     data_plane_api_keys: Optional list of Bearer tokens for data plane auth.
///     health_check: Enable periodic health checking of worker nodes (default True).
///     health_check_interval_secs: Interval in seconds between health check rounds (default 60).
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
    #[pyo3(get)]
    policy: String,
    #[pyo3(get)]
    pd_mode: Option<String>,
    #[pyo3(get)]
    prefill_urls: Vec<String>,
    #[pyo3(get)]
    decode_urls: Vec<String>,
    #[pyo3(get)]
    data_plane_api_keys: Vec<String>,
    #[pyo3(get)]
    health_check: bool,
    #[pyo3(get)]
    health_check_interval_secs: u64,
    #[pyo3(get)]
    log_file: Option<String>,
    // ── K8s service discovery ───────────────────────────────────────────
    #[pyo3(get)]
    k8s_selector: Vec<String>,
    #[pyo3(get)]
    k8s_namespace: Option<String>,
    #[pyo3(get)]
    k8s_port: u16,
    #[pyo3(get)]
    k8s_check_interval_secs: u64,
    #[pyo3(get)]
    k8s_prefill_selector: Vec<String>,
    #[pyo3(get)]
    k8s_decode_selector: Vec<String>,
}

#[pymethods]
impl Router {
    #[new]
    #[pyo3(signature = (worker_urls, host="0.0.0.0", port=30000, request_timeout_secs=300, log_level="info", policy="least-loaded", pd_mode=None, prefill_urls=None, decode_urls=None, data_plane_api_keys=None, health_check=true, health_check_interval_secs=60, log_file=None, k8s_selector=None, k8s_namespace=None, k8s_port=8000, k8s_check_interval_secs=60, k8s_prefill_selector=None, k8s_decode_selector=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        worker_urls: Vec<String>,
        host: &str,
        port: u16,
        request_timeout_secs: u64,
        log_level: &str,
        policy: &str,
        pd_mode: Option<&str>,
        prefill_urls: Option<Vec<String>>,
        decode_urls: Option<Vec<String>>,
        data_plane_api_keys: Option<Vec<String>>,
        health_check: bool,
        health_check_interval_secs: u64,
        log_file: Option<String>,
        k8s_selector: Option<Vec<String>>,
        k8s_namespace: Option<String>,
        k8s_port: u16,
        k8s_check_interval_secs: u64,
        k8s_prefill_selector: Option<Vec<String>>,
        k8s_decode_selector: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let policy = validate_policy(policy)?;
        let pd_mode = match pd_mode {
            Some(s) if s.to_lowercase() == "vllm" || s.to_lowercase() == "sglang" => {
                Some(s.to_string())
            }
            Some(s) => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Invalid pd_mode '{}', expected 'vllm' or 'sglang'",
                    s
                )))
            }
            None => None,
        };
        Ok(Router {
            worker_urls,
            host: host.to_string(),
            port,
            request_timeout_secs,
            log_level: log_level.to_string(),
            policy,
            pd_mode,
            prefill_urls: prefill_urls.unwrap_or_default(),
            decode_urls: decode_urls.unwrap_or_default(),
            data_plane_api_keys: data_plane_api_keys.unwrap_or_default(),
            health_check,
            health_check_interval_secs,
            log_file,
            k8s_selector: k8s_selector.unwrap_or_default(),
            k8s_namespace,
            k8s_port,
            k8s_check_interval_secs,
            k8s_prefill_selector: k8s_prefill_selector.unwrap_or_default(),
            k8s_decode_selector: k8s_decode_selector.unwrap_or_default(),
        })
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
        let policy = self.policy.clone();
        let pd_mode = self.pd_mode.clone();
        let prefill_urls = self.prefill_urls.clone();
        let decode_urls = self.decode_urls.clone();
        let data_plane_api_keys = self.data_plane_api_keys.clone();
        let health_check = self.health_check;
        let health_check_interval_secs = self.health_check_interval_secs;
        // K8s fields
        let k8s_selector = self.k8s_selector.clone();
        let k8s_namespace = self.k8s_namespace.clone();
        let k8s_port = self.k8s_port;
        let k8s_check_interval_secs = self.k8s_check_interval_secs;
        let k8s_prefill_selector = self.k8s_prefill_selector.clone();
        let k8s_decode_selector = self.k8s_decode_selector.clone();
        let is_k8s = !k8s_selector.is_empty()
            || !k8s_prefill_selector.is_empty()
            || !k8s_decode_selector.is_empty();

        // Release the GIL before blocking on the async server loop.
        let log_file_owned = self.log_file.clone();
        let result: Result<(), String> = py.allow_threads(move || {
            let init_result =
                router::config::logging::setup_tracing(
                    EnvFilter::new(level),
                    log_file_owned.as_deref(),
                );

            match init_result {
                Ok(()) => {}
                Err(_) => {
                    eprintln!(
                        "[router] tracing subscriber already set; logs may go to a different subscriber"
                    );
                }
            }

            let num_api_keys = data_plane_api_keys.len();
            tracing::info!(
                host = %ip,
                port = addr.port(),
                worker_urls = ?worker_urls,
                prefill_urls = ?prefill_urls,
                decode_urls = ?decode_urls,
                pd_mode = ?pd_mode,
                policy = %policy,
                health_check = health_check,
                health_check_interval_secs = health_check_interval_secs,
                data_plane_auth = num_api_keys > 0,
                "Starting router (Python bindings)",
            );

            let rt = tokio::runtime::Runtime::new().map_err(|e| {
                format!("Failed to create Tokio runtime: {}", e)
            })?;

            let state: Arc<AppState> = if is_k8s {
                // ── K8s service discovery mode ────────────────────────────
                let policy_type = match policy.as_str() {
                    "least-loaded" => config::Policy::LeastLoaded,
                    "power-of-two" => config::Policy::PowerOfTwo,
                    "random" => config::Policy::Random,
                    "round-robin" => config::Policy::RoundRobin,
                    "session-affinity" => config::Policy::SessionAffinity,
                    "prefix-affinity" => config::Policy::PrefixAffinity,
                    "load-cache-aware" => config::Policy::LoadCacheAware,
                    _ => config::Policy::LeastLoaded,
                };

                let parse_sel = |items: &[String]| -> HashMap<String, String> {
                    config::parse_selector(items)
                };

                if let Some(ref pd_mode_val) = pd_mode {
                    let pd_mode_enum = match pd_mode_val.to_lowercase().as_str() {
                        "vllm" => PdMode::Vllm,
                        "sglang" => PdMode::Sglang,
                        _ => unreachable!(),
                    };

                    let prefill_pool: SharedWorkerPool =
                        Arc::new(std::sync::RwLock::new(Vec::new()));
                    let decode_pool: SharedWorkerPool =
                        Arc::new(std::sync::RwLock::new(Vec::new()));

                    let state = Arc::new(AppState::new_k8s_pd(
                        pd_mode_enum,
                        timeout_secs,
                        policy_type,
                        Arc::clone(&prefill_pool),
                        Arc::clone(&decode_pool),
                    ));

                    let sd_config = ServiceDiscoveryConfig {
                        enabled: true,
                        selector: parse_sel(&k8s_selector),
                        check_interval: Duration::from_secs(k8s_check_interval_secs),
                        port: k8s_port,
                        namespace: k8s_namespace.clone(),
                        pd_mode: true,
                        prefill_selector: parse_sel(&k8s_prefill_selector),
                        decode_selector: parse_sel(&k8s_decode_selector),
                    };

                    rt.spawn(async move {
                        match service_discovery::start_service_discovery(
                            sd_config,
                            Arc::new(std::sync::RwLock::new(Vec::new())),
                            Some(prefill_pool),
                            Some(decode_pool),
                        )
                        .await
                        {
                            Ok(handle) => {
                                let _ = handle.await;
                            }
                            Err(e) => {
                                tracing::error!("Failed to start service discovery: {e}");
                            }
                        }
                    });

                    state
                } else {
                    let workers_pool: SharedWorkerPool =
                        Arc::new(std::sync::RwLock::new(Vec::new()));

                    let state = Arc::new(AppState::new_k8s(
                        timeout_secs,
                        policy_type,
                        Arc::clone(&workers_pool),
                    ));

                    let sd_config = ServiceDiscoveryConfig {
                        enabled: true,
                        selector: parse_sel(&k8s_selector),
                        check_interval: Duration::from_secs(k8s_check_interval_secs),
                        port: k8s_port,
                        namespace: k8s_namespace.clone(),
                        pd_mode: false,
                        prefill_selector: Default::default(),
                        decode_selector: Default::default(),
                    };

                    rt.spawn(async move {
                        match service_discovery::start_service_discovery(
                            sd_config,
                            workers_pool,
                            None,
                            None,
                        )
                        .await
                        {
                            Ok(handle) => {
                                let _ = handle.await;
                            }
                            Err(e) => {
                                tracing::error!("Failed to start service discovery: {e}");
                            }
                        }
                    });

                    state
                }
            } else {
                // ── Static worker mode ────────────────────────────────────
                if let Some(ref pd_mode_val) = pd_mode {
                    assert!(
                        !prefill_urls.is_empty(),
                        "PD mode requires at least one prefill_urls"
                    );
                    assert!(
                        !decode_urls.is_empty(),
                        "PD mode requires at least one decode_urls"
                    );

                    let pd_mode_enum = match pd_mode_val.to_lowercase().as_str() {
                        "vllm" => PdMode::Vllm,
                        "sglang" => PdMode::Sglang,
                        _ => unreachable!(),
                    };

                    let n_prefill = prefill_urls.len();
                    let n_decode = decode_urls.len();

                    Arc::new(AppState::new_pd(
                        pd_mode_enum,
                        Worker::from_urls(&prefill_urls),
                        Worker::from_urls(&decode_urls),
                        timeout_secs,
                        make_policy(&policy, n_prefill),
                        make_policy(&policy, n_decode),
                    ))
                } else {
                    let n = worker_urls.len();
                    Arc::new(AppState::new(
                        Worker::from_urls(&worker_urls),
                        timeout_secs,
                        make_policy(&policy, n),
                    ))
                }
            };

            // Spawn background health-check task when enabled.
            if health_check {
                let interval = Duration::from_secs(health_check_interval_secs);
                let health_client = state.client.clone();
                let state_ref = Arc::clone(&state);

                rt.spawn(async move {
                    tracing::info!(
                        interval_secs = interval.as_secs(),
                        "Health check task started",
                    );
                    loop {
                        tokio::time::sleep(interval).await;

                        if state_ref.is_k8s_mode() {
                            let regular = state_ref.k8s_workers_snapshot();
                            let prefill = state_ref.k8s_prefill_snapshot();
                            let decode = state_ref.k8s_decode_snapshot();

                            if !regular.is_empty() {
                                worker::run_health_checks(&health_client, &regular).await;
                            }
                            if !prefill.is_empty() {
                                worker::run_health_checks(&health_client, &prefill).await;
                            }
                            if !decode.is_empty() {
                                worker::run_health_checks(&health_client, &decode).await;
                            }
                        } else {
                            let health_workers = state_ref.workers.clone();
                            let health_prefill_workers = state_ref.prefill_workers().to_vec();
                            let health_decode_workers = state_ref.decode_workers().to_vec();

                            worker::run_health_checks(&health_client, &health_workers).await;
                            if !health_prefill_workers.is_empty() {
                                worker::run_health_checks(&health_client, &health_prefill_workers).await;
                            }
                            if !health_decode_workers.is_empty() {
                                worker::run_health_checks(&health_client, &health_decode_workers).await;
                            }
                        }
                    }
                });
            }

            let auth_config = AuthConfig::new(Some(data_plane_api_keys));
            let app = server::build_router(state, auth_config);

            rt.block_on(async move {
                tracing::info!("Listening on {}", addr);

                let listener = TcpListener::bind(addr).await.map_err(|e| {
                    format!("Failed to bind to {}: {}", addr, e)
                })?;

                axum::serve(listener, app)
                    .with_graceful_shutdown(server::shutdown_signal())
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
