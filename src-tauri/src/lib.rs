// Tauri 2 entry point. Wires up:
//   - DuckDB connection (managed as Tauri State)
//   - Migrations (run once at startup)
//   - Seed hook (intentionally a no-op — roster comes from Sling)
//   - Stronghold plugin (OS-keychain-backed vault for the Sling token)
//   - Anthropic API key + Sling token (in-memory state caches; Stronghold
//     write for the Sling token is delegated to the frontend via the
//     plugin's JS bridge)
//   - IPC commands (exposed to the React frontend)

mod algorithm;
mod commands;
mod editor;
mod db;
mod logging;
mod migrations;
mod review;
mod secrets;
mod seed;
mod sling;
mod sling_login;

use std::sync::Mutex;

use commands::{AnthropicKey, SlingOrgHint, SlingToken};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // File logging + panic hook before anything else. Plugin initialization
    // and the event-loop/webview bootstrap all run BEFORE the setup hook, so
    // a failure there would otherwise die with nothing in the log (Windows
    // discards stderr).
    logging::early_init();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_stronghold::Builder::new(|password| {
            // Stronghold vault encryption key. v1 uses a static derivation
            // baked into the binary — adequate for single-user local-only
            // use per the spec. Future: derive from OS user credentials.
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(b"barrekeep-v1-vault-key:");
            hasher.update(password.as_bytes());
            hasher.finalize().to_vec()
        }).build())
        .setup(|app| {
            // First: confirm the log path now that an AppHandle exists (the
            // panic hook is already installed by early_init).
            logging::init(app.handle());
            let result = setup_app(app);
            // A setup error aborts startup — record it here, because the
            // generic "failed to setup" surfaced by Tauri never reaches the
            // log on Windows.
            match &result {
                Ok(()) => logging::write_line("startup", "setup complete; creating main window"),
                Err(e) => logging::write_line("startup", &format!("setup FAILED: {e}")),
            }
            result
        })
        .invoke_handler(tauri::generate_handler![
            logging::log_frontend_error,
            commands::db_info,
            commands::list_teachers,
            commands::update_teacher_settings,
            commands::list_positions,
            commands::set_position_active,
            commands::list_qualified_pairs,
            commands::generate_proposal,
            commands::list_proposals,
            commands::get_proposal,
            commands::edit_proposal_shift_teacher,
            commands::edit_proposal_shift_position,
            commands::list_edits_for_proposal,
            commands::has_sling_token,
            commands::set_anthropic_key,
            commands::has_anthropic_key,
            commands::get_app_setting,
            commands::set_app_setting,
            commands::list_algorithm_versions,
            commands::adopt_algorithm_version,
            commands::delete_algorithm_script,
            commands::set_sling_token,
            commands::set_sling_credentials,
            commands::has_sling_credentials,
            commands::get_studio_config,
            commands::set_studio_config,
            commands::open_sling_login_window,
            commands::discover_studio_config,
            commands::review_proposal,
            commands::claude_edit_proposal,
            commands::claude_draft_code_change,
            commands::validate_code_draft,
            commands::list_reviews_for_proposal,
            commands::pull_month_from_sling,
            commands::refresh_roster_from_sling,
            commands::import_external_shift,
            commands::list_availability_blocks,
            commands::list_external_shifts_for_month,
            commands::push_proposal_dry_run,
            commands::push_proposal_execute,
        ])
        .run(tauri::generate_context!());

    // Covers failures outside setup: plugin init, window/webview creation,
    // event-loop errors. `.expect()` alone would print to the discarded
    // stderr on Windows.
    if let Err(e) = result {
        logging::write_line("fatal", &format!("tauri run failed: {e}"));
        std::process::exit(1);
    }
}

/// Everything the app wires up at startup, with a log line per stage so the
/// last "startup" line in barrekeep.log names the stage that died.
fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(desktop)]
    {
        logging::write_line("startup", "registering updater + process plugins");
        app.handle()
            .plugin(tauri_plugin_updater::Builder::new().build())?;
        app.handle().plugin(tauri_plugin_process::init())?;
    }
    let path = db::db_path(app.handle())?;
    logging::write_line("startup", &format!("opening database at {}", path.display()));
    let db = db::Db::open(app.handle())?;
    {
        let conn = db.0.lock().expect("db poisoned at startup");
        if let Some(backup) = migrations::backup_if_pending(&conn, &path)? {
            logging::write_line("migration", &format!("backed up database to {}", backup.display()));
        }
        logging::write_line("startup", "running migrations");
        migrations::run(&conn)?;
        seed::run_if_empty(&conn)?;
        logging::write_line("startup", "migrations + seed complete; algorithm archive sweep");
        // Tidy old algorithm script versions (spec: >3 versions
        // behind AND unused >3 months → algorithms/archive/).
        match algorithm::algorithms_dir(app.handle()) {
            Ok(dir) => match algorithm::archive_sweep(&conn, &dir) {
                Ok(moved) => {
                    for f in moved {
                        logging::write_line("algorithm", &format!("archived old script {f}"));
                    }
                }
                Err(e) => logging::write_line("algorithm", &format!("archive sweep failed: {e}")),
            },
            Err(e) => logging::write_line("algorithm", &format!("no algorithms dir: {e}")),
        }
    }
    app.manage(db);

    // Open the Stronghold-backed secrets vault and preload the
    // Sling token (if any). If the vault can't be opened for any
    // reason, log and continue with no token rather than killing
    // app startup.
    logging::write_line("startup", "opening secrets vault");
    let (secrets, initial_token, initial_anthropic) =
        match secrets::Secrets::open(&app.handle()) {
            Ok(s) => {
                let tok = s.get(secrets::KEY_SLING_TOKEN).unwrap_or_else(|e| {
                    logging::write_line("secrets", &format!("failed to read sling_token: {e}"));
                    None
                });
                let anthropic = s.get(secrets::KEY_ANTHROPIC).unwrap_or_else(|e| {
                    logging::write_line("secrets", &format!("failed to read anthropic_key: {e}"));
                    None
                });
                (Some(s), tok, anthropic)
            }
            Err(e) => {
                logging::write_line("secrets", &format!("failed to open vault: {e}"));
                (None, None, None)
            }
        };
    if let Some(s) = secrets {
        app.manage(s);
    }
    app.manage(AnthropicKey(Mutex::new(initial_anthropic)));
    app.manage(SlingToken(Mutex::new(initial_token)));
    app.manage(SlingOrgHint(Mutex::new(None)));
    Ok(())
}
