//! In-app browser-login flow for Sling.
//!
//! Opens a Tauri webview window pointed at app.getsling.com with an
//! injected interceptor script. When the page makes its first
//! authenticated request to api.getsling.com, the script triggers a
//! same-origin navigation to `/__bk_capture?t=<token>`. The Rust-side
//! `on_navigation` hook intercepts that URL, extracts the token into the
//! in-memory SlingToken state, then hands persistence + window-close off
//! to a background thread.
//!
//! Why navigation rather than a Tauri event emit: Tauri 2 does not
//! expose the IPC bridge on external (remote) URLs by default, so a
//! JS-emitted event would silently no-op. Navigation interception fires
//! on WebKitGTK / WebView2 / WKWebView with no capability config.
//!
//! IMPORTANT: the `on_navigation` callback runs on the UI thread *inside*
//! the webview's navigation event. Doing blocking I/O or window-lifecycle
//! work (close) there deadlocks WebView2 on Windows (reentrancy into the
//! controller mid-event). WebKitGTK tolerated it, so the bug only surfaced
//! on Windows. Anything beyond a cheap in-memory write must be deferred to
//! run after the handler returns — see the handler body.
//!
//! See docs/superpowers/specs/2026-05-20-sling-browser-login-design.md.

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::commands::SlingToken;
use crate::secrets::{Secrets, KEY_SLING_EMAIL, KEY_SLING_PASSWORD};

const LABEL: &str = "sling-login";
const CAPTURE_HOST: &str = "app.getsling.com";
const CAPTURE_PATH: &str = "/__bk_capture";

/// Opens (or focuses, if already open) the Sling login webview window.
pub fn open_login_window(app: AppHandle) -> Result<()> {
    const CAPTURE_SCRIPT: &str = include_str!("sling_login_capture.js");

    if let Some(existing) = app.get_webview_window(LABEL) {
        existing.set_focus().ok();
        return Ok(());
    }

    // Pull saved credentials (if any) from Stronghold and inject them into
    // the page via a JS preamble. The autofill block in sling_login_capture.js
    // reads `window.__BK_CREDS` and fills the form. Captcha + submit are
    // left to the user.
    let creds_preamble = build_creds_preamble(&app);
    let full_script = format!("{creds_preamble}\n{CAPTURE_SCRIPT}");

    let fired = Arc::new(AtomicBool::new(false));

    let app_for_nav = app.clone();
    let fired_for_nav = fired.clone();
    let _window = WebviewWindowBuilder::new(
        &app,
        LABEL,
        WebviewUrl::External("https://app.getsling.com/".parse()?),
    )
    .title("Sign in to Sling")
    .inner_size(1100.0, 800.0)
    // Match the UA we send from ureq in sling.rs so Cloudflare WAF sees
    // a consistent client across login + subsequent API pulls. Especially
    // relevant on Linux WebKitGTK, whose default UA differs from Chrome.
    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
    .initialization_script(&full_script)
    .on_navigation(move |url| {
        // Allow every navigation except our sentinel.
        if url.host_str() != Some(CAPTURE_HOST) || url.path() != CAPTURE_PATH {
            return true;
        }
        // Atomically claim the capture; ignore duplicates.
        if fired_for_nav.swap(true, Ordering::SeqCst) {
            return false;
        }
        let Some(token) = url
            .query_pairs()
            .find(|(k, _)| k == "t")
            .map(|(_, v)| v.into_owned())
        else {
            eprintln!("[sling_login] sentinel URL had no 't' query param");
            return false;
        };
        // Store the in-memory token now — this is a cheap, non-blocking mutex
        // write and is safe to do inside the handler.
        if let Some(state) = app_for_nav.try_state::<SlingToken>() {
            if let Ok(mut guard) = state.0.lock() {
                *guard = Some(token.clone());
            }
        }
        // Stash the org hint (cheap mutex write — safe inside on_navigation).
        if let Some(org) = url.query_pairs()
            .find(|(k, _)| k == "o")
            .and_then(|(_, v)| v.parse::<i64>().ok())
        {
            if let Some(hint) = app_for_nav.try_state::<crate::commands::SlingOrgHint>() {
                if let Ok(mut g) = hint.0.lock() { *g = Some(org); }
            }
        }
        // CRITICAL — Windows / WebView2: do NOT perform window-lifecycle
        // (close) or blocking I/O (Stronghold save) synchronously inside
        // on_navigation. This callback runs on the UI thread *inside* the
        // webview's NavigationStarting event; calling close() re-enters the
        // event loop to tear down the WebView2 controller mid-event and
        // deadlocks the whole app (the main window shares this thread).
        // WebKitGTK tolerates it, which is why it only froze on Windows.
        // Defer everything to a thread that runs AFTER this handler returns,
        // and close via run_on_main_thread so it lands on the next event-loop
        // tick rather than reentrantly.
        let app_deferred = app_for_nav.clone();
        std::thread::spawn(move || {
            // Persist off the UI thread (Stronghold save = disk I/O + crypto).
            if let Some(secrets) = app_deferred.try_state::<crate::secrets::Secrets>() {
                if let Err(e) = secrets.set(crate::secrets::KEY_SLING_TOKEN, &token) {
                    eprintln!("[sling_login] failed to persist token: {e}");
                }
            }
            let _ = app_deferred.emit("sling-token-saved", ());
            let app_close = app_deferred.clone();
            let _ = app_deferred.run_on_main_thread(move || {
                if let Some(w) = app_close.get_webview_window(LABEL) {
                    let _ = w.close();
                }
            });
        });
        false
    })
    .build()?;

    // When the user closes the window without capturing, emit cancel.
    if let Some(w) = app.get_webview_window(LABEL) {
        let app_for_cancel = app.clone();
        let fired_for_cancel = fired.clone();
        w.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed)
                && !fired_for_cancel.load(Ordering::SeqCst)
            {
                let _ = app_for_cancel.emit("sling-login-cancelled", ());
            }
        });
    }

    Ok(())
}

/// Build a JS preamble that sets `window.__BK_CREDS` from Stronghold-saved
/// credentials. Always returns at least the marker comment so the script
/// is syntactically valid even if no credentials are saved.
fn build_creds_preamble(app: &AppHandle) -> String {
    let Some(secrets) = app.try_state::<Secrets>() else {
        return "/* no secrets vault */".to_string();
    };
    let email = secrets.get(KEY_SLING_EMAIL).ok().flatten().unwrap_or_default();
    let password = secrets.get(KEY_SLING_PASSWORD).ok().flatten().unwrap_or_default();
    if email.is_empty() && password.is_empty() {
        return "/* no creds saved */".to_string();
    }
    let json = serde_json::json!({ "email": email, "password": password });
    // serde_json::to_string emits a JSON string literal — safe for direct
    // embedding in a JS source as long as it isn't followed by </script>,
    // which can't appear here (no <script> tags in capture flow).
    format!("window.__BK_CREDS = {};", json)
}
