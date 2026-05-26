pub mod auth;
pub mod logging;

use clap::{ArgAction, Parser};

/// Load-balancing policy selection.
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum Policy {
    /// Least-loaded (default) — selects the worker with fewest in-flight requests.
    LeastLoaded,
    /// Power of two choices — picks two random workers, selects the one with fewer in-flight requests.
    PowerOfTwo,
    /// Random — picks a worker uniformly at random.
    Random,
    /// Round-robin — cycles through workers sequentially.
    RoundRobin,
    /// Session affinity — deterministic hash on user/session_id so multi-turn
    /// conversations always land on the same worker.
    SessionAffinity,
    /// Prefix affinity — hash on first-message prefix with queue-depth
    /// threshold; falls back to join-shortest-queue when the preferred
    /// worker is overloaded.
    PrefixAffinity,
    /// Load + cache-aware scoring — balances cache affinity (who has the
    /// prefix/session cached) against current load using configurable
    /// `alpha`/`beta` weights.
    LoadCacheAware,
}

/// Prefill-Decode separation mode.
///
/// `None` (default) means regular mode where all workers handle both prefill and decode.
/// `Some(Vllm)` enables vLLM PD separation with sequential two-stage processing.
/// `Some(Sglang)` enables SGLang PD separation with concurrent dual dispatch.
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum PdMode {
    /// vLLM Prefill-Decode separation mode.
    Vllm,
    /// SGLang Prefill-Decode separation mode (concurrent dual dispatch).
    Sglang,
}

/// Log level for filtering tracing output.
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum LogLevel {
    /// Only errors.
    Error,
    /// Warnings and errors.
    Warn,
    /// Info, warnings, and errors (default).
    Info,
    /// Everything, including debug.
    Debug,
}

impl LogLevel {
    /// Convert to the equivalent `tracing::Level`.
    pub fn to_tracing_level(&self) -> tracing::Level {
        match self {
            LogLevel::Error => tracing::Level::ERROR,
            LogLevel::Warn => tracing::Level::WARN,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Debug => tracing::Level::DEBUG,
        }
    }
}

/// Configuration for the router gateway.
///
/// Parsed from CLI arguments and/or environment variables via clap.
#[derive(Parser, Debug, Clone)]
#[command(name = "router", version)]
pub struct Config {
    /// Host address to bind to.
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    pub host: String,

    /// Port to listen on.
    #[arg(long, env = "PORT", default_value = "30000")]
    pub port: u16,

    /// Comma-separated worker URLs (e.g. "http://192.168.1.10:8000 http://192.168.1.20:8000").
    #[arg(long, num_args = 0..)]
    pub worker_urls: Vec<String>,

    /// Maximum time in seconds to wait for a worker response.
    #[arg(long, env = "REQUEST_TIMEOUT", default_value = "300")]
    pub request_timeout_secs: u64,

    /// Load-balancing policy (least-loaded, power-of-two, random, round-robin).
    #[arg(long, env = "POLICY", default_value = "least-loaded")]
    pub policy: Policy,

    /// Minimum log level (error, warn, info, debug).
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: LogLevel,

    /// Enable Prefill-Decode separation mode ("vllm" or "sglang").
    #[arg(long, env = "PD_MODE")]
    pub pd_mode: Option<PdMode>,

    /// Comma-separated prefill worker URLs for PD mode.
    #[arg(long, num_args = 0..)]
    pub prefill_urls: Vec<String>,

    /// Comma-separated decode worker URLs for PD mode.
    #[arg(long, num_args = 0..)]
    pub decode_urls: Vec<String>,

    /// API keys for data plane access (format: key)
    #[arg(long = "data-plane-api-keys", action = ArgAction::Append, env = "DATA_PLANE_API_KEYS", value_delimiter = ',', help_heading = "Data Plane Authentication")]
    pub data_plane_api_keys: Vec<String>,

    /// Enable periodic health checking of worker nodes.
    #[arg(long, env = "HEALTH_CHECK", default_value = "true", action = ArgAction::Set)]
    pub health_check: bool,

    /// Interval in seconds between health check rounds.
    #[arg(long, env = "HEALTH_CHECK_INTERVAL", default_value = "60")]
    pub health_check_interval_secs: u64,

    /// Optional file path for JSON log output. If not set, logs only go to stderr.
    #[arg(long, env = "LOG_FILE")]
    pub log_file: Option<String>,
}
