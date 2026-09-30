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

/// SQL expression rendering a TIMESTAMPTZ column as an ISO-8601 UTC string,
/// `2026-11-01T06:30:00Z` (NULL stays NULL). Use it for EVERY TIMESTAMPTZ the
/// app reads as text: `CAST(ts AS VARCHAR)` renders in the session time zone
/// — UTC without ICU, local time with it — so its meaning depends on which
/// extensions happen to be loaded. Only core functions (epoch_us →
/// make_timestamp → strftime), so it's ICU-independent. The frontend parses
/// the result with `new Date(..)` and shows local time.
///
/// A macro (not a fn) so it can be spliced into `const` SQL with `concat!`.
macro_rules! utc_iso {
    ($col:literal) => {
        concat!("strftime(make_timestamp(epoch_us(", $col, ")), '%Y-%m-%dT%H:%M:%SZ')")
    };
}
pub(crate) use utc_iso;

pub fn open_file<P: AsRef<std::path::Path>>(path: P) -> duckdb::Result<Connection> {
    Connection::open_with_flags(path, config()?)
}

/// In-memory connection with the app's config (tests, and the placeholder
/// that holds the Db slot while backup::create_backup copies the file).
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

    #[test]
    fn utc_iso_renders_timestamptz_as_utc_z() {
        let c = super::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE t (id INTEGER, ts TIMESTAMPTZ);
             INSERT INTO t VALUES
               (1, TIMESTAMPTZ '2026-11-01 01:30:00-05:00'),
               (2, CAST('2026-11-02T05:00:00.250-06:00' AS TIMESTAMPTZ)),
               (3, NULL);",
        )
        .unwrap();
        let got: Vec<Option<String>> = c
            .prepare(concat!("SELECT ", utc_iso!("ts"), " FROM t ORDER BY id"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            got,
            vec![
                Some("2026-11-01T06:30:00Z".to_string()),
                Some("2026-11-02T11:00:00Z".to_string()),
                None,
            ]
        );
    }
}
