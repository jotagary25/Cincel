//! Sends `tracing` (and the `log` records GPUI and wgpu emit) to stderr, as
//! before Etapa 4, and additionally to a daily-rotating file under a log
//! directory the caller resolves (`~/.local/state/cincel/log/`, or
//! `$CINCEL_LOG_DIR` for tests — `crate::paths::state_dir` plus `"log"` is
//! the caller's job, not this crate's, so it stays independent of
//! `cincel-settings`).

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer as _;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// How many rotated log files are kept (`docs/specs/06-etapa4-conexiones-y-cincel.md`
/// §8: "rotación diaria (7 archivos)").
const KEPT_LOG_FILES: usize = 7;

/// File name the rotating appender bases its daily files on, e.g.
/// `cincel.2026-09-26.log`.
const LOG_FILE_PREFIX: &str = "cincel";
const LOG_FILE_SUFFIX: &str = "log";

/// The `tracing` target of the timed sections of startup and project opening
/// (`docs/specs/08-etapa6-cierre-1-0.md` §3.3); `cincel-workspace` names it
/// `bench::TIMING_TARGET`.
const TIMING_TARGET: &str = "cincel::timing";

/// Environment variable that turns the timed sections on.
const TIMINGS_VAR: &str = "CINCEL_TRACE_TIMINGS";

/// Whether `CINCEL_TRACE_TIMINGS` asks for the timed sections: `1`, `true`
/// or `yes`.
fn timings_enabled(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes")
    )
}

/// `CINCEL_TRACE_TIMINGS=1`: one stderr line per timed section when it
/// closes (`close time.busy=… time.idle=…`; the two add up to its whole
/// life), whatever `RUST_LOG` says, and nothing else from this layer.
fn timings_layer<S>() -> Option<impl tracing_subscriber::Layer<S>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    timings_enabled(std::env::var(TIMINGS_VAR).ok().as_deref()).then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_span_events(FmtSpan::CLOSE)
            .with_filter(Targets::new().with_target(TIMING_TARGET, tracing::Level::TRACE))
    })
}

/// Builds the filter: `rust_log` (the `RUST_LOG` environment variable) wins;
/// `log_level` (`--log-level`) is the fallback; `info` is the default.
/// Separated from [`build_filter`] so tests can pick `rust_log` directly
/// instead of racing the process environment.
fn filter_from(rust_log: Option<&str>, log_level: Option<&str>) -> EnvFilter {
    if let Some(rust_log) = rust_log
        && let Ok(filter) = EnvFilter::try_new(rust_log)
    {
        return filter;
    }
    EnvFilter::try_new(log_level.unwrap_or("info")).unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Built fresh for each layer rather than cloned: both must agree on the
/// same result, but `EnvFilter` is not `Clone`.
fn build_filter(log_level: Option<&str>) -> EnvFilter {
    filter_from(std::env::var("RUST_LOG").ok().as_deref(), log_level)
}

/// Sets up logging for the whole process.
///
/// `log_dir` is created if it does not exist. When it cannot be created or
/// opened for writing, logging falls back to stderr only (with a message
/// explaining why): a broken log directory must never stop the editor from
/// starting.
///
/// The returned guard owns the background thread that flushes the file
/// writer; the caller must keep it alive for the whole process (`main` holds
/// it in a local binding until it returns). `None` means the file layer
/// could not be set up; stderr logging is still active either way.
pub fn init(log_level: Option<&str>, log_dir: &Path) -> Option<WorkerGuard> {
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_filter(build_filter(log_level));
    let file_setup = std::fs::create_dir_all(log_dir)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix(LOG_FILE_PREFIX)
                .filename_suffix(LOG_FILE_SUFFIX)
                .max_log_files(KEPT_LOG_FILES)
                .build(log_dir)
                .map_err(|error| error.to_string())
        });

    match file_setup {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            // Log files have no terminal to render ANSI colors in.
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_target(true)
                .with_filter(build_filter(log_level));
            tracing_subscriber::registry()
                .with(stderr_layer)
                .with(file_layer)
                .with(timings_layer())
                .init();
            Some(guard)
        }
        Err(error) => {
            tracing_subscriber::registry()
                .with(stderr_layer)
                .with(timings_layer())
                .init();
            eprintln!(
                "cincel: no se pudo abrir el log en {} ({error}); solo se escribe en stderr",
                log_dir.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `tracing_subscriber::registry().init()` sets a *global* subscriber, so
    // it can only be exercised once per process; these tests stick to the
    // file-system half of `init` (creating the directory, building the
    // appender) rather than calling `init` itself, which every other test in
    // this binary would also race against.

    #[test]
    fn creates_the_log_directory_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let log_dir = dir.path().join("state/cincel/log");
        assert!(!log_dir.exists());

        std::fs::create_dir_all(&log_dir).unwrap();
        let appender = RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix(LOG_FILE_PREFIX)
            .filename_suffix(LOG_FILE_SUFFIX)
            .max_log_files(KEPT_LOG_FILES)
            .build(&log_dir);

        assert!(appender.is_ok(), "{appender:?}");
        assert!(log_dir.is_dir());
    }

    #[test]
    fn timings_are_off_unless_asked_for() {
        assert!(!timings_enabled(None));
        assert!(!timings_enabled(Some("")));
        assert!(!timings_enabled(Some("0")));
        assert!(timings_enabled(Some("1")));
        assert!(timings_enabled(Some(" TRUE ")));
    }

    #[test]
    fn defaults_to_info_with_neither_source() {
        assert_eq!(filter_from(None, None).to_string(), "info");
    }

    #[test]
    fn falls_back_to_the_explicit_log_level() {
        assert_eq!(filter_from(None, Some("debug")).to_string(), "debug");
    }

    #[test]
    fn rust_log_wins_over_the_explicit_log_level() {
        assert_eq!(filter_from(Some("warn"), Some("debug")).to_string(), "warn");
    }
}
