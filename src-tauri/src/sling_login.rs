//! In-app browser-login flow for Sling.
//!
//! Opens a Tauri webview window pointed at app.getsling.com with an
//! injected interceptor script. When the page makes its first
//! authenticated request to api.getsling.com, the script triggers a
//! same-origin navigation to `/__bk_capture?t=<token>`. The Rust-side
//! `on_navigation` hook intercepts that URL, extracts the token, saves
//! it into the SlingToken state, emits `sling-token-saved`, and closes
//! the window.
//!
//! Why navigation rather than a Tauri event emit: Tauri 2 does not
//! expose the IPC bridge on external (remote) URLs by default, so a
//! JS-emitted event would silently no-op. Navigation interception works
//! identically on WebKitGTK / WebView2 / WKWebView with no capability
//! config.
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
        if let Some(state) = app_for_nav.try_state::<SlingToken>() {
            if let Ok(mut guard) = state.0.lock() {
                *guard = Some(token.clone());
            }
        }
        // Persist to Stronghold so the token survives app restarts.
        if let Some(secrets) = app_for_nav.try_state::<crate::secrets::Secrets>() {
            if let Err(e) = secrets.set(crate::secrets::KEY_SLING_TOKEN, &token) {
                eprintln!("[sling_login] failed to persist token: {e}");
            }
        }
        let _ = app_for_nav.emit("sling-token-saved", ());
        if let Some(w) = app_for_nav.get_webview_window(LABEL) {
            let _ = w.close();
        }
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
