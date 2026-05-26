//! Size-based log rotation with gzip compression.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use flate2::write::GzEncoder;
use flate2::Compression;

const DEFAULT_MAX_SIZE: u64 = 5 * 1024 * 1024; // 5 MB

/// A file writer that automatically rotates when the file exceeds `max_size` bytes.
///
/// Old log content is gzip-compressed with a timestamp suffix:
/// `<original>.<YYYY-MM-DD>T<HH-MM-SS>.gz`
#[derive(Clone)]
pub struct RotatingFileWriter {
    inner: Arc<Mutex<RotatingInner>>,
}

struct RotatingInner {
    file: Option<File>,
    bytes_written: u64,
    max_size: u64,
    file_path: PathBuf,
}

impl RotatingFileWriter {
    /// Open `path` for appending. Creates parent directories and the file if
    /// they don't exist. Rotation triggers when the file exceeds `max_size` bytes.
    pub fn new(path: impl AsRef<Path>, max_size: u64) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes_written = file.metadata()?.len();
        Ok(Self {
            inner: Arc::new(Mutex::new(RotatingInner {
                file: Some(file),
                bytes_written,
                max_size,
                file_path: path,
            })),
        })
    }

    /// Open `path` with the default 5 MB rotation threshold.
    pub fn with_default_max(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::new(path, DEFAULT_MAX_SIZE)
    }
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self.inner.lock().unwrap();
        if inner.bytes_written + buf.len() as u64 > inner.max_size && inner.bytes_written > 0 {
            inner.rotate()?;
        }
        let n = inner.file.as_mut().unwrap().write(buf)?;
        inner.bytes_written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        match inner.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl RotatingInner {
    fn rotate(&mut self) -> io::Result<()> {
        // Close the current file handle.
        let mut file = self.file.take().unwrap();
        file.flush()?;
        drop(file);

        // Read current content, compress to backup.
        let content = fs::read(&self.file_path).unwrap_or_default();
        if !content.is_empty() {
            let compressed = gzip_compress(&content)?;
            let backup_path = make_backup_path(&self.file_path);
            fs::write(&backup_path, compressed)?;
        }

        // Truncate the log file for fresh writes.
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.file_path)?,
        );
        self.bytes_written = 0;

        Ok(())
    }
}

fn make_backup_path(original: &Path) -> PathBuf {
    let ts = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S");
    let name = original
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("router.log");
    original.with_file_name(format!("{}.{}.gz", name, ts))
}

fn gzip_compress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    encoder.finish()
}

/// Initialize the tracing subscriber with a terminal layer (stderr, human-readable)
/// and an optional JSON file layer with rotation.
///
/// Always uses [`try_init`](tracing_subscriber::util::SubscriberInitExt::try_init);
/// callers that want a panic on failure should `.expect()` the result.
pub fn setup_tracing(
    env_filter: tracing_subscriber::EnvFilter,
    log_file: Option<&str>,
) -> Result<(), String> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let terminal_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    let result = if let Some(path) = log_file {
        let rotating = RotatingFileWriter::with_default_max(path)
            .map_err(|e| format!("Failed to create log file: {}", e))?;
        let (non_blocking, guard) = tracing_appender::non_blocking(rotating);
        Box::leak(Box::new(guard));

        let file_layer = tracing_subscriber::fmt::layer()
            .json()
            .with_writer(non_blocking);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(terminal_layer)
            .with(file_layer)
            .try_init()
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(terminal_layer)
            .try_init()
    };

    result.map_err(|e| format!("Subscriber already initialized: {}", e))
}
