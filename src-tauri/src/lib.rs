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
                let path = db::db_path(app.handle())?;
                if let Some(backup) = migrations::backup_if_pending(&conn, &path)? {
                    eprintln!("[migration] backed up database to {}", backup.display());
                }
                migrations::run(&conn)?;
                seed::run_if_empty(&conn)?;
                // Tidy old algorithm script versions (spec: >3 versions
                // behind AND unused >3 months → algorithms/archive/).
                match algorithm::algorithms_dir(app.handle()) {
                    Ok(dir) => match algorithm::archive_sweep(&conn, &dir) {
                        Ok(moved) => {
                            for f in moved {
                                eprintln!("[algorithm] archived old script {f}");
                            }
                        }
                        Err(e) => eprintln!("[algorithm] archive sweep failed: {e}"),
                    },
                    Err(e) => eprintln!("[algorithm] no algorithms dir: {e}"),
                }
            }
            app.manage(db);

            // Open the Stronghold-backed secrets vault and preload the
            // Sling token (if any). If the vault can't be opened for any
            // reason, log and continue with no token rather than killing
            // app startup.
            let (secrets, initial_token, initial_anthropic) =
                match secrets::Secrets::open(&app.handle()) {
                    Ok(s) => {
                        let tok = s.get(secrets::KEY_SLING_TOKEN).unwrap_or_else(|e| {
                            eprintln!("[secrets] failed to read sling_token: {e}");
                            None
                        });
                        let anthropic = s.get(secrets::KEY_ANTHROPIC).unwrap_or_else(|e| {
                            eprintln!("[secrets] failed to read anthropic_key: {e}");
                            None
                        });
                        (Some(s), tok, anthropic)
                    }
                    Err(e) => {
                        eprintln!("[secrets] failed to open vault: {e}");
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
        })
        .invoke_handler(tauri::generate_handler![
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
            commands::list_reviews_for_proposal,
            commands::pull_month_from_sling,
            commands::refresh_roster_from_sling,
            commands::import_external_shift,
            commands::list_availability_blocks,
            commands::list_external_shifts_for_month,
            commands::push_proposal_dry_run,
            commands::push_proposal_execute,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
