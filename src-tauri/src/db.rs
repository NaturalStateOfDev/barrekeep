use std::path::PathBuf;
use std::sync::Mutex;

use duckdb::Connection;
use tauri::{AppHandle, Manager};

/// Wraps the DuckDB connection in a Mutex so Tauri can hold it as State.
/// DuckDB's Connection isn't Sync, but it is Send, so a Mutex is enough
/// for the single-window single-user shape of this app.
pub struct Db(pub Mutex<Connection>);

impl Db {
    pub fn open(app: &AppHandle) -> anyhow::Result<Self> {
        let path = db_path(app)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        Ok(Db(Mutex::new(conn)))
    }
}

/// Resolved DB file path. Lives in the app's local data dir so it survives
/// reinstalls. the maintainer can find it at:
///   Windows: %LOCALAPPDATA%\com.barrekeep.app\scheduler.duckdb
pub fn db_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| anyhow::anyhow!("could not resolve app_local_data_dir: {e}"))?;
    Ok(dir.join("scheduler.duckdb"))
}
