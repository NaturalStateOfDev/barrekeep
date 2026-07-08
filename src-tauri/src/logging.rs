//! Lightweight file logging so failures are diagnosable on Windows, where a
//! windowed app's stderr is discarded (see CLAUDE.md — "eprintln!/stderr does
//! NOT reliably reach" anything on Windows). Everything is appended to
//! `<app_log_dir>/barrekeep.log`:
//!   Windows: %LOCALAPPDATA%\com.barrekeep.app\logs\barrekeep.log
//!   Linux:   ~/.local/share/com.barrekeep.app/logs/barrekeep.log
//!
//! Captures both sides of the app:
//!   - Rust panics, via a panic hook installed in `init`.
//!   - Frontend errors, via the `log_frontend_error` command, wired from
//!     window.onerror / unhandledrejection and a React ErrorBoundary.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::{AppHandle, Manager};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Resolve and remember the log file path, install a panic hook, and record a
/// startup line. Call once, first thing in the setup hook.
pub fn init(app: &AppHandle) {
    if let Ok(dir) = app.path().app_log_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = LOG_PATH.set(dir.join("barrekeep.log"));
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        write_line("PANIC", &format!("{info}\n{backtrace}"));
        previous(info);
    }));
    write_line(
        "startup",
        &format!("Barrekeep {} starting", env!("CARGO_PKG_VERSION")),
    );
}

/// Append one timestamped line to the log file. Never panics; a no-op if the
/// path was never resolved.
pub fn write_line(source: &str, message: &str) {
    let Some(path) = LOG_PATH.get() else { return };
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{ts}] {source}: {message}");
    }
}

/// Persist a frontend error (from window.onerror / an ErrorBoundary) into the
/// same log file.
#[tauri::command]
pub fn log_frontend_error(message: String) {
    write_line("frontend", &message);
}
