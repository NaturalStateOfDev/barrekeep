//! Lightweight file logging so failures are diagnosable on Windows, where a
//! windowed app's stderr is discarded (see CLAUDE.md — "eprintln!/stderr does
//! NOT reliably reach" anything on Windows). Everything is appended to
//! `<app_log_dir>/barrekeep.log`:
//!   Windows: %LOCALAPPDATA%\com.barrekeep.app\logs\barrekeep.log
//!   Linux:   ~/.local/share/com.barrekeep.app/logs/barrekeep.log
//!
//! Captures both sides of the app:
//!   - Rust panics, via a panic hook installed in `early_init`.
//!   - Frontend errors, via the `log_frontend_error` command, wired from the
//!     inline handlers in index.html and a React ErrorBoundary.
//!
//! `early_init` runs before the Tauri builder so failures during plugin
//! initialization and the event-loop/webview bootstrap — all of which happen
//! before the `setup` hook — are still recorded. Reading the log after a
//! startup crash: the last "startup" line names the stage that died; an
//! empty/absent log means the process never reached `run()` (loader-level
//! failure — missing DLL, broken WebView2 install).

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::{AppHandle, Manager};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Must match `identifier` in tauri.conf.json — lets us find the log dir
/// before Tauri's path resolver exists.
const APP_IDENTIFIER: &str = "com.barrekeep.app";

/// Set up logging with no AppHandle, first thing in `run()`: resolve the log
/// path from environment variables (mirroring Tauri's app_log_dir), install
/// the panic hook, and record process + webview-runtime info.
pub fn early_init() {
    if let Some(dir) = fallback_log_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = LOG_PATH.set(dir.join("barrekeep.log"));
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        write_line("PANIC", &format!("{info}\n{backtrace}"));
        previous(info);
    }));
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("<unknown: {e}>"));
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    write_line(
        "startup",
        &format!(
            "Barrekeep {} process start ({profile}, {}/{}), exe={exe}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        ),
    );
    // On Windows an Err here means the WebView2 runtime is missing or broken
    // — the classic cause of an instant exit / blank frame with no other
    // symptom.
    match tauri::webview_version() {
        Ok(v) => write_line("startup", &format!("webview runtime version: {v}")),
        Err(e) => write_line("startup", &format!("webview runtime NOT detected: {e}")),
    }
}

/// Where Tauri's app_log_dir will land, computed without an AppHandle.
fn fallback_log_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join(APP_IDENTIFIER).join("logs"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|base| base.join(APP_IDENTIFIER).join("logs"))
    }
}

/// Confirm the log path against Tauri's real resolver once an AppHandle
/// exists. Call first thing in the setup hook.
pub fn init(app: &AppHandle) {
    match app.path().app_log_dir() {
        Ok(dir) => {
            let _ = std::fs::create_dir_all(&dir);
            let resolved = dir.join("barrekeep.log");
            if LOG_PATH.set(resolved.clone()).is_err() && LOG_PATH.get() != Some(&resolved) {
                write_line(
                    "startup",
                    &format!(
                        "note: tauri resolves app_log_dir to {} but early lines are in this file",
                        dir.display()
                    ),
                );
            }
        }
        Err(e) => write_line("startup", &format!("could not resolve app_log_dir: {e}")),
    }
    write_line("startup", "setup hook entered");
}

/// Append one timestamped line to the log file, mirrored to stderr for dev
/// terminals (stderr is unreliable on Windows; the file is the source of
/// truth). Never panics; the file write is a no-op if the path was never
/// resolved.
pub fn write_line(source: &str, message: &str) {
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = writeln!(std::io::stderr(), "[{ts}] {source}: {message}");
    let Some(path) = LOG_PATH.get() else { return };
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
