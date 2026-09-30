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
        let conn = open_file(&path)?;
        Ok(Db(Mutex::new(conn)))
    }
}

/// Connection config for every connection the app opens. The json extension
/// is compiled in (duckdb crate `json` feature); icu is not available from
/// the published crate, so the SQL avoids ICU-only functions (e.g.
/// `epoch(TIMESTAMPTZ)` → `epoch_us`). Autoinstall/autoload are OFF so a
/// query that slips an ICU dependency in fails loudly in tests instead of
/// silently downloading from extensions.duckdb.org on the studio PC.
fn config() -> duckdb::Result<duckdb::Config> {
    duckdb::Config::default().enable_autoload_extension(false)
}

pub fn open_file<P: AsRef<std::path::Path>>(path: P) -> duckdb::Result<Connection> {
    Connection::open_with_flags(path, config()?)
}

/// In-memory connection with the app's config (tests).
#[cfg(test)]
pub fn open_in_memory() -> duckdb::Result<Connection> {
    Connection::open_in_memory_with_flags(config()?)
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

#[cfg(test)]
mod tests {
    #[test]
    fn json_extension_is_compiled_in() {
        let c = super::open_in_memory().unwrap();
        let loaded: bool = c
            .query_row(
                "SELECT loaded FROM duckdb_extensions() WHERE extension_name = 'json'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(loaded, "json should be statically linked (duckdb `json` feature)");
        // With autoload off, a json function binding proves no download is needed.
        let v: i64 = c
            .query_row("SELECT CAST(json_extract('{\"a\": 2}', '$.a') AS BIGINT)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
    }
}
