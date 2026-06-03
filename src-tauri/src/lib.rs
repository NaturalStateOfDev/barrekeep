// Tauri 2 entry point. Wires up:
//   - DuckDB connection (managed as Tauri State)
//   - Migrations (run once at startup)
//   - Seed data (populates roster + positions on first launch)
//   - Stronghold plugin (OS-keychain-backed vault for the Sling token)
//   - Anthropic API key + Sling token (in-memory state caches; Stronghold
//     write for the Sling token is delegated to the frontend via the
//     plugin's JS bridge)
//   - IPC commands (exposed to the React frontend)

mod commands;
mod db;
mod migrations;
mod review;
mod secrets;
mod seed;
mod sling;
mod sling_login;

use std::sync::Mutex;

use commands::{AnthropicKey, SlingToken};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            #[cfg(desktop)]
            {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
            }
            let db = db::Db::open(app.handle())?;
            {
                let conn = db.0.lock().expect("db poisoned at startup");
                migrations::run(&conn)?;
                seed::run_if_empty(&conn)?;
            }
            app.manage(db);

            // Open the Stronghold-backed secrets vault and preload the
            // Sling token (if any). If the vault can't be opened for any
            // reason, log and continue with no token rather than killing
            // app startup.
            let (secrets, initial_token) = match secrets::Secrets::open(&app.handle()) {
                Ok(s) => {
                    let tok = s.get(secrets::KEY_SLING_TOKEN).unwrap_or_else(|e| {
                        eprintln!("[secrets] failed to read sling_token: {e}");
                        None
                    });
                    (Some(s), tok)
                }
                Err(e) => {
                    eprintln!("[secrets] failed to open vault: {e}");
                    (None, None)
                }
            };
            if let Some(s) = secrets {
                app.manage(s);
            }
            app.manage(AnthropicKey(Mutex::new(None)));
            app.manage(SlingToken(Mutex::new(initial_token)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::db_info,
            commands::list_teachers,
            commands::list_sling_candidates,
            commands::update_teacher_settings,
            commands::list_positions,
            commands::list_qualified_pairs,
            commands::generate_proposal,
            commands::list_proposals,
            commands::get_proposal,
            commands::edit_proposal_shift_teacher,
            commands::list_edits_for_proposal,
            commands::has_sling_token,
            commands::set_anthropic_key,
            commands::has_anthropic_key,
            commands::set_sling_token,
            commands::set_sling_credentials,
            commands::has_sling_credentials,
            commands::get_studio_config,
            commands::set_studio_config,
            commands::open_sling_login_window,
            commands::review_proposal,
            commands::list_reviews_for_proposal,
            commands::pull_month_from_sling,
            commands::import_external_shift,
            commands::list_availability_blocks,
            commands::list_external_shifts_for_month,
            commands::add_teacher_from_pull,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
