use clap::Parser;

/// Configuration for the tokendog-router gateway.
///
/// Parsed from CLI arguments and/or environment variables via clap.
#[derive(Parser, Debug, Clone)]
#[command(name = "tokendog-router", version)]
pub struct Config {
    /// Address to listen on (e.g. "127.0.0.1:8000").
    #[arg(short, long, env = "LISTEN_ADDR", default_value = "127.0.0.1:8000")]
    pub listen_addr: String,

    /// Comma-separated backend URLs (e.g. "http://192.168.1.10:8000,http://192.168.1.20:8000").
    #[arg(short, long, env = "BACKENDS", required = true, value_delimiter = ',')]
    pub backends: Vec<String>,

    /// Maximum time in seconds to wait for a backend response.
    #[arg(long, env = "REQUEST_TIMEOUT", default_value = "300")]
    pub request_timeout_secs: u64,
}
