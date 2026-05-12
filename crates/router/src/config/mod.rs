use clap::Parser;

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
}
