//! In-process ring buffer of client log lines.
//!
//! Captures the `tracing` events the desktop app emits (relay client, store,
//! Tauri commands) plus errors forwarded from the webview, and exposes them to
//! the UI through the `get_client_logs` / `append_client_log` commands. The
//! buffer is bounded ([`MAX_LOG_LINES`]) so a long session never grows without
//! limit, and every entry keeps its level and target so the Logs tab can
//! filter and render a terminal-style listing.
//!
//! Per AGENTS.md the app never logs message content or keys — logs are
//! peer-ID and target level only — so capturing every line is safe.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::Subscriber;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

/// Maximum number of log lines kept in memory before the oldest are dropped.
pub const MAX_LOG_LINES: usize = 2000;

/// A single captured log line, as surfaced to the Logs tab.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// Epoch milliseconds when the line was written.
    pub timestamp: u64,
    /// Uppercase level name: TRACE, DEBUG, INFO, WARN or ERROR.
    pub level: String,
    /// The tracing target (module path) that produced the line; `"webview"`
    /// for errors forwarded from the frontend.
    pub target: String,
    /// The formatted message (fields rendered as `message key=value ...`).
    pub message: String,
}

/// The shared, bounded log storage behind the buffer.
#[derive(Default)]
struct LogBufferInner {
    lines: VecDeque<LogEntry>,
}

/// Thread-safe, bounded log ring buffer shared with the tracing layer and the
/// Tauri commands via [`Arc`]. Cheap to clone; every clone writes into the
/// same storage.
#[derive(Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<LogBufferInner>>,
}

impl LogBuffer {
    /// Create a fresh, empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a formatted line, dropping the oldest entry when the buffer is
    /// full. Lock failures are swallowed: logging must never panic the app.
    pub fn write_line(&self, level: &str, target: &str, message: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.lines.len() >= MAX_LOG_LINES {
                inner.lines.pop_front();
            }
            inner.lines.push_back(LogEntry {
                timestamp: now_millis(),
                level: level.to_string(),
                target: target.to_string(),
                message: message.to_string(),
            });
        }
    }

    /// Snapshot the newest entries; `limit` (when given) caps the length.
    pub fn snapshot(&self, limit: Option<usize>) -> Vec<LogEntry> {
        let Ok(guard) = self.inner.lock() else {
            return Vec::new();
        };
        let count = limit.unwrap_or(MAX_LOG_LINES).min(guard.lines.len());
        let start = guard.lines.len() - count;
        guard.lines.range(start..).cloned().collect()
    }
}

/// A [`tracing::Layer`] that mirrors every event into the shared buffer.
#[derive(Clone)]
pub struct CaptureLayer {
    buffer: LogBuffer,
}

impl CaptureLayer {
    /// Build a capture layer writing into `buffer`.
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut fields = FieldCapture::default();
        event.record(&mut fields);
        self.buffer.write_line(
            level_name(metadata.level()),
            metadata.target(),
            &fields.render(),
        );
    }
}

/// Collects an event's fields so they can be rendered as plain text: the
/// `message` field carries the formatted message, every other field becomes a
/// trailing `key=value` pair.
#[derive(Default)]
struct FieldCapture {
    message: Option<String>,
    extras: Vec<(String, String)>,
}

impl FieldCapture {
    /// Render the captured fields as `message key=value ...`.
    fn render(&self) -> String {
        match (&self.message, self.extras.is_empty()) {
            (Some(message), true) => message.clone(),
            (Some(message), false) => {
                let extras = self
                    .extras
                    .iter()
                    .map(|(key, value)| format!(" {key}={value}"))
                    .collect::<String>();
                format!("{message}{extras}")
            }
            (None, _) => self
                .extras
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

impl tracing::field::Visit for FieldCapture {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let value = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.extras.push((field.name().to_string(), value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.extras
                .push((field.name().to_string(), value.to_string()));
        }
    }
}

/// The uppercase name of a tracing level.
fn level_name(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::TRACE => "TRACE",
        tracing::Level::DEBUG => "DEBUG",
        tracing::Level::INFO => "INFO",
        tracing::Level::WARN => "WARN",
        tracing::Level::ERROR => "ERROR",
    }
}

/// Formats timestamps as local wall-clock RFC 3339 (`2026-08-06T16:48:00.123Z`
/// style, but with the local UTC offset) so logs match the user's clock.
struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        use time::format_description::well_known::Rfc3339;
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let rendered = now.format(&Rfc3339).unwrap_or_else(|_| "?".into());
        w.write_str(&rendered)
    }
}

/// Initialize `tracing` for the app: every event is written to stderr (dev
/// console), mirrored into the shared ring buffer that backs the Logs settings
/// tab, and — when `log_dir` is provided — appended to a daily file
/// (`whisper-YYYY-MM-DD.log`) so users can share complete error logs in bug
/// reports. Call once from the Tauri setup hook; a second call is a no-op
/// (`try_init` returns the existing default subscriber unchanged).
pub fn init_tracing(buffer: &LogBuffer, log_dir: Option<PathBuf>) {
    // Local wall-clock timestamps (not UTC), so logs match the user's clock.
    let timer = LocalTimer;
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_timer(timer)
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_filter(LevelFilter::INFO);
    let capture_layer = CaptureLayer::new(buffer.clone()).with_filter(LevelFilter::INFO);
    let registry = tracing_subscriber::registry()
        .with(stderr_layer)
        .with(capture_layer);
    if let Some(dir) = log_dir {
        if let Ok(file) = open_daily_log(&dir) {
            // Same format as stderr so a pasted file matches the Logs tab, but
            // WITHOUT ANSI colours — the escape codes would make the file
            // unreadable in a plain text editor / GitHub issue.
            let file_layer = tracing_subscriber::fmt::layer()
                .with_timer(LocalTimer)
                .with_writer(file)
                .with_target(true)
                .with_ansi(false)
                .with_filter(LevelFilter::INFO);
            let _ = registry.with(file_layer).try_init();
            return;
        }
    }
    let _ = registry.try_init();
}

/// Open (appending) the daily log file `logs/whisper-YYYY-MM-DD.log`,
/// creating the directory first. One file per day per process start; a long
/// running app keeps appending to the day it started.
fn open_daily_log(dir: &Path) -> std::io::Result<std::fs::File> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("whisper-{}.log", date_stamp()));
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// The current UTC date as `YYYY-MM-DD` (no external chrono dependency).
fn date_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    // Days since 1970-01-01 → civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Current time as epoch milliseconds.
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_line_then_snapshot_roundtrips_entries() {
        let buffer = LogBuffer::new();
        buffer.write_line("INFO", "whisper::relay", "connected");
        buffer.write_line("ERROR", "whisper::relay", "failed");

        let entries = buffer.snapshot(None);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].level, "INFO");
        assert_eq!(entries[0].target, "whisper::relay");
        assert_eq!(entries[0].message, "connected");
        assert_eq!(entries[1].level, "ERROR");
        assert_eq!(entries[1].message, "failed");
        // Timestamps are epoch-millis in the past or now, never zero.
        assert!(entries[0].timestamp > 0);
        assert!(entries[1].timestamp >= entries[0].timestamp);
    }

    #[test]
    fn buffer_drops_the_oldest_line_beyond_capacity() {
        let buffer = LogBuffer::new();
        for index in 0..(MAX_LOG_LINES + 25) {
            buffer.write_line("DEBUG", "t", &format!("line {index}"));
        }
        let entries = buffer.snapshot(None);
        assert_eq!(entries.len(), MAX_LOG_LINES);
        // The newest lines survive; the first 25 were evicted.
        assert_eq!(entries[0].message, "line 25");
        assert_eq!(
            entries[MAX_LOG_LINES - 1].message,
            format!("line {}", MAX_LOG_LINES + 24)
        );
    }

    #[test]
    fn snapshot_limit_returns_only_the_newest_lines() {
        let buffer = LogBuffer::new();
        for index in 0..10 {
            buffer.write_line("INFO", "t", &format!("line {index}"));
        }
        let entries = buffer.snapshot(Some(3));
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message, "line 7");
        assert_eq!(entries[2].message, "line 9");

        // A limit larger than the buffer is clamped to the buffer length.
        assert_eq!(buffer.snapshot(Some(500)).len(), 10);
    }

    #[test]
    fn level_name_maps_every_tracing_level() {
        assert_eq!(level_name(&tracing::Level::TRACE), "TRACE");
        assert_eq!(level_name(&tracing::Level::DEBUG), "DEBUG");
        assert_eq!(level_name(&tracing::Level::INFO), "INFO");
        assert_eq!(level_name(&tracing::Level::WARN), "WARN");
        assert_eq!(level_name(&tracing::Level::ERROR), "ERROR");
    }

    #[test]
    fn field_capture_renders_message_and_extras() {
        let capture = FieldCapture {
            message: Some("relay unreachable".to_string()),
            extras: vec![("attempt".to_string(), "3".to_string())],
        };
        assert_eq!(capture.render(), "relay unreachable attempt=3");
    }

    #[test]
    fn field_capture_renders_extras_without_a_message() {
        let capture = FieldCapture {
            message: None,
            extras: vec![("peer".to_string(), "alice".to_string())],
        };
        assert_eq!(capture.render(), "peer=alice");
    }

    #[test]
    fn capture_layer_tees_events_into_the_buffer() {
        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(CaptureLayer::new(buffer.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(peer = "alice", "connected to relay");
        });

        let entries = buffer.snapshot(None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, "INFO");
        assert_eq!(entries[0].target, module_path!());
        assert_eq!(entries[0].message, "connected to relay peer=alice");
    }
}
