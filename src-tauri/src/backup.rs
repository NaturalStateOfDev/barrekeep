//! Routine rotating backups of the live database.
//!
//! Files land in `<app_local_data>/backups/scheduler-YYYYMMDD-HHMMSS-<reason>.duckdb`
//! (next to scheduler.duckdb). Taken on startup (at most once per calendar
//! day), before every Sling push, and on demand from Settings. The newest
//! KEEP backups are kept; rotation only ever deletes files matching that
//! exact name pattern — nothing else in the folder is touched.
//!
//! The copy is made THROUGH the open connection with DuckDB's own
//! `ATTACH … ; COPY FROM DATABASE …` rather than by copying the file: on
//! Windows, copying scheduler.duckdb while the app holds it open fails with a
//! sharing violation (os error 32) — see migrations::backup_if_pending. The
//! result is an ordinary DuckDB file that opens on its own.
//!
//! A failed backup never blocks startup or a push: it is logged to
//! barrekeep.log, remembered in BackupState, and surfaced as a warning.
//!
//! Restore is manual (docs/architecture.md → "Restoring a backup").

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{NaiveDate, NaiveDateTime};
use duckdb::Connection;
use serde::Serialize;
use tauri::State;

use crate::db::Db;

/// How many routine backups to keep.
pub const KEEP: usize = 14;
const PREFIX: &str = "scheduler-";
const SUFFIX: &str = ".duckdb";
const PARTIAL: &str = ".partial";
const ATTACH_ALIAS: &str = "barrekeep_backup";

/// Last backup failure (cleared by the next success), for the UI warning.
#[derive(Default)]
pub struct BackupState(pub Mutex<Option<String>>);

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct BackupEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    /// Local time the backup was taken, "YYYY-MM-DD HH:MM:SS".
    pub created_at: String,
    pub reason: String,
}

pub fn backups_dir(db_file: &Path) -> PathBuf {
    db_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join("backups")
}

/// Reasons are folded to [a-z0-9] so the file name stays parseable.
fn sanitize_reason(reason: &str) -> String {
    let r: String = reason
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if r.is_empty() { "manual".into() } else { r }
}

pub fn backup_file_name(at: NaiveDateTime, reason: &str) -> String {
    format!("{PREFIX}{}-{}{SUFFIX}", at.format("%Y%m%d-%H%M%S"), sanitize_reason(reason))
}

/// Parse a name produced by backup_file_name. Anything else → None (and is
/// therefore never listed or rotated away).
pub fn parse_backup_name(name: &str) -> Option<(NaiveDateTime, String)> {
    let core = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    // "YYYYMMDD-HHMMSS-reason"
    if core.len() < 17 || core.as_bytes().get(15) != Some(&b'-') {
        return None;
    }
    let (stamp, rest) = core.split_at(15);
    let reason = &rest[1..];
    if reason.is_empty() || !reason.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return None;
    }
    let at = NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").ok()?;
    Some((at, reason.to_string()))
}

/// All well-formed backups in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<BackupEntry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<(NaiveDateTime, BackupEntry)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let (at, reason) = parse_backup_name(&name)?;
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((
                at,
                BackupEntry {
                    path: e.path().display().to_string(),
                    name,
                    size_bytes: meta.len(),
                    created_at: at.format("%Y-%m-%d %H:%M:%S").to_string(),
                    reason,
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.name.cmp(&a.1.name)));
    out.into_iter().map(|(_, e)| e).collect()
}

/// Delete all but the newest `keep` well-formed backups. Returns the names
/// deleted. Files not matching the backup name pattern are never touched.
pub fn rotate(dir: &Path, keep: usize) -> anyhow::Result<Vec<String>> {
    let mut deleted = Vec::new();
    for old in list(dir).into_iter().skip(keep) {
        std::fs::remove_file(&old.path)?;
        deleted.push(old.name);
    }
    Ok(deleted)
}

/// Has a backup with this reason already been taken on `day`?
pub fn has_backup_on(dir: &Path, day: NaiveDate, reason: &str) -> bool {
    let reason = sanitize_reason(reason);
    list(dir).iter().any(|b| {
        parse_backup_name(&b.name).is_some_and(|(at, r)| at.date() == day && r == reason)
    })
}

fn sql_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn remove_partial(partial: &Path) {
    let _ = std::fs::remove_file(partial);
    let mut wal = partial.as_os_str().to_owned();
    wal.push(".wal");
    let _ = std::fs::remove_file(PathBuf::from(wal));
}

/// Copy the connection's main database into a new backup file in `dir`.
/// Writes to a `.partial` name and renames on success, so an interrupted
/// backup never shows up as a real one.
pub fn create_backup(
    conn: &Connection,
    dir: &Path,
    reason: &str,
    at: NaiveDateTime,
) -> anyhow::Result<BackupEntry> {
    std::fs::create_dir_all(dir)?;
    // Two backups in the same second get a numeric reason suffix.
    let base = sanitize_reason(reason);
    let mut name = backup_file_name(at, &base);
    let mut n = 2;
    while dir.join(&name).exists() {
        name = backup_file_name(at, &format!("{base}{n}"));
        n += 1;
    }
    let final_path = dir.join(&name);
    let partial = dir.join(format!("{name}{PARTIAL}"));
    remove_partial(&partial);

    let main_db: String = conn.query_row("SELECT current_database()", [], |r| r.get(0))?;
    let partial_str = partial
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("backup path is not valid UTF-8"))?;

    let copy = (|| -> anyhow::Result<()> {
        conn.execute_batch(&format!("ATTACH {} AS {ATTACH_ALIAS};", sql_string(partial_str)))?;
        let copied = conn.execute_batch(&format!(
            "COPY FROM DATABASE \"{}\" TO {ATTACH_ALIAS};",
            main_db.replace('"', "\"\"")
        ));
        // Always detach — even after a failed COPY — so the partial file is
        // closed before we delete it (Windows won't delete an open file).
        let detached = conn.execute_batch(&format!("DETACH {ATTACH_ALIAS};"));
        copied?;
        detached?;
        Ok(())
    })();
    if let Err(e) = copy {
        let _ = conn.execute_batch(&format!("DETACH DATABASE IF EXISTS {ATTACH_ALIAS};"));
        remove_partial(&partial);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&partial, &final_path) {
        remove_partial(&partial);
        return Err(e.into());
    }
    let size_bytes = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    Ok(BackupEntry {
        path: final_path.display().to_string(),
        name,
        size_bytes,
        created_at: at.format("%Y-%m-%d %H:%M:%S").to_string(),
        reason: parse_backup_name(&final_path.file_name().unwrap_or_default().to_string_lossy())
            .map(|(_, r)| r)
            .unwrap_or(base),
    })
}

/// Take a backup + rotate, logging either way and recording the outcome in
/// `state`. Never panics; returns a user-facing warning on failure.
pub fn run(
    conn: &Connection,
    db_file: &Path,
    reason: &str,
    state: Option<&BackupState>,
) -> Result<BackupEntry, String> {
    let dir = backups_dir(db_file);
    let result = create_backup(conn, &dir, reason, chrono::Local::now().naive_local());
    let outcome = match result {
        Ok(entry) => {
            crate::logging::write_line(
                "backup",
                &format!("{reason} backup written: {} ({} bytes)", entry.path, entry.size_bytes),
            );
            match rotate(&dir, KEEP) {
                Ok(deleted) => {
                    for d in deleted {
                        crate::logging::write_line("backup", &format!("rotated out {d}"));
                    }
                }
                Err(e) => crate::logging::write_line("backup", &format!("rotation failed: {e:#}")),
            }
            Ok(entry)
        }
        Err(e) => {
            let msg = format!("{reason} backup failed: {e:#}");
            crate::logging::write_line("backup", &msg);
            Err(msg)
        }
    };
    if let Some(state) = state {
        if let Ok(mut last) = state.0.lock() {
            *last = outcome.as_ref().err().cloned();
        }
    }
    outcome
}

/// Startup hook: one "startup" backup per calendar day (local date).
pub fn run_startup(conn: &Connection, db_file: &Path, state: &BackupState) {
    let dir = backups_dir(db_file);
    let today = chrono::Local::now().date_naive();
    if has_backup_on(&dir, today, "startup") {
        return;
    }
    let _ = run(conn, db_file, "startup", Some(state));
}

#[derive(Serialize)]
pub struct BackupsInfo {
    pub dir: String,
    pub keep: usize,
    pub backups: Vec<BackupEntry>,
    /// Most recent backup failure this session, if the last attempt failed.
    pub last_error: Option<String>,
}

#[tauri::command]
pub fn list_backups(
    app: tauri::AppHandle,
    state: State<'_, BackupState>,
) -> Result<BackupsInfo, String> {
    let db_file = crate::db::db_path(&app).map_err(|e| e.to_string())?;
    let dir = backups_dir(&db_file);
    Ok(BackupsInfo {
        dir: dir.display().to_string(),
        keep: KEEP,
        backups: list(&dir),
        last_error: state.0.lock().ok().and_then(|g| g.clone()),
    })
}

#[tauri::command]
pub fn backup_now(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    state: State<'_, BackupState>,
) -> Result<BackupEntry, String> {
    let db_file = crate::db::db_path(&app).map_err(|e| e.to_string())?;
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    run(&conn, &db_file, "manual", Some(&state))
}

/// Open the backups folder in the OS file manager (created if missing).
#[tauri::command]
pub fn open_backups_folder(app: tauri::AppHandle) -> Result<(), String> {
    let db_file = crate::db::db_path(&app).map_err(|e| e.to_string())?;
    let dir = backups_dir(&db_file);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // explorer.exe returns exit code 1 even on success, so only a failure to
    // launch counts as an error.
    std::process::Command::new(program)
        .arg(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "barrekeep-backup-test-{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn names_round_trip() {
        let n = backup_file_name(at("2026-09-29 14:05:09"), "startup");
        assert_eq!(n, "scheduler-20260929-140509-startup.duckdb");
        assert_eq!(parse_backup_name(&n), Some((at("2026-09-29 14:05:09"), "startup".into())));
        assert_eq!(backup_file_name(at("2026-09-29 14:05:09"), "Pre-Push!"), "scheduler-20260929-140509-prepush.duckdb");
        for bad in [
            "scheduler.duckdb",
            "scheduler.duckdb.backup-v9",
            "scheduler-20260929-140509-startup.duckdb.partial",
            "scheduler-20260929-140509-.duckdb",
            "scheduler-20260929-140509-Start.duckdb",
            "scheduler-2026099-140509-startup.duckdb",
            "scheduler-20261329-140509-startup.duckdb",
            "notes.txt",
        ] {
            assert_eq!(parse_backup_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn backup_of_live_db_reopens_with_data_and_sequences() {
        let dir = tmpdir("copy");
        let db_file = dir.join("scheduler.duckdb");
        let conn = Connection::open(&db_file).unwrap();
        crate::migrations::run(&conn).unwrap();
        for m in ["2026-10", "2026-11", "2026-12"] {
            conn.execute(
                "INSERT INTO proposals (target_month, algorithm_version, parameters, is_current)
                 VALUES (?, 'v9', '{}', TRUE)",
                duckdb::params![m],
            )
            .unwrap();
        }
        // A write still sitting in the WAL (no checkpoint) must be included.
        let max_id: i64 = conn.query_row("SELECT max(id) FROM proposals", [], |r| r.get(0)).unwrap();

        let bdir = backups_dir(&db_file);
        let entry = create_backup(&conn, &bdir, "manual", at("2026-09-29 10:00:00")).unwrap();
        assert_eq!(entry.name, "scheduler-20260929-100000-manual.duckdb");
        assert!(entry.size_bytes > 0);
        // Live connection is still usable and nothing is left attached.
        let n: i64 = conn.query_row("SELECT count(*) FROM proposals", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        let attached: i64 = conn
            .query_row(
                "SELECT count(*) FROM duckdb_databases() WHERE database_name = ?",
                duckdb::params![ATTACH_ALIAS],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attached, 0);
        // Only the finished backup is left behind: no .partial, no .wal.
        let files: Vec<String> = std::fs::read_dir(&bdir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(files, vec![entry.name.clone()]);
        drop(conn);

        // The backup opens on its own and has the data + schema version.
        let b = Connection::open(&entry.path).unwrap();
        let n: i64 = b.query_row("SELECT count(*) FROM proposals", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        assert_eq!(
            crate::migrations::current_version(&b).unwrap(),
            crate::migrations::MIGRATIONS.last().unwrap().version
        );
        // Sequences carry on past the copied ids, so a restored backup can
        // keep inserting without primary-key collisions.
        let next_id: i64 = b
            .query_row(
                "INSERT INTO proposals (target_month, algorithm_version, parameters, is_current)
                 VALUES ('2027-01', 'v9', '{}', TRUE) RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(next_id > max_id, "sequence restarted: {next_id} <= {max_id}");
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_second_backups_get_distinct_names() {
        let dir = tmpdir("dup");
        let conn = Connection::open(dir.join("scheduler.duckdb")).unwrap();
        crate::migrations::run(&conn).unwrap();
        let bdir = dir.join("backups");
        let t = at("2026-09-29 10:00:00");
        let a = create_backup(&conn, &bdir, "manual", t).unwrap();
        let b = create_backup(&conn, &bdir, "manual", t).unwrap();
        assert_ne!(a.name, b.name);
        assert_eq!(b.name, "scheduler-20260929-100000-manual2.duckdb");
        assert_eq!(list(&bdir).len(), 2);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_keeps_newest_14_and_ignores_other_files() {
        let dir = tmpdir("rotate");
        let base = at("2026-09-01 08:00:00");
        for i in 0..20 {
            let name = backup_file_name(base + chrono::Duration::hours(i * 13), "startup");
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let others = [
            "scheduler.duckdb",
            "scheduler.duckdb.backup-v9",
            "scheduler-20200101-000000-old.duckdb.partial",
            "my-notes.txt",
            "scheduler-20200101-000000-Manual.duckdb",
        ];
        for o in others {
            std::fs::write(dir.join(o), b"keep me").unwrap();
        }
        let before = list(&dir);
        assert_eq!(before.len(), 20);
        let deleted = rotate(&dir, KEEP).unwrap();
        assert_eq!(deleted.len(), 6);
        let after = list(&dir);
        assert_eq!(after.len(), KEEP);
        // Newest first, and exactly the newest 14 survived.
        assert_eq!(after, before[..KEEP].to_vec());
        for d in &deleted {
            assert!(before[KEEP..].iter().any(|b| &b.name == d));
        }
        for o in others {
            assert!(dir.join(o).exists(), "{o} must never be rotated away");
        }
        // Idempotent.
        assert!(rotate(&dir, KEEP).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_backup_once_per_day() {
        let dir = tmpdir("daily");
        let day = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        assert!(!has_backup_on(&dir, day, "startup"));
        std::fs::write(dir.join(backup_file_name(at("2026-09-29 07:00:00"), "prepush")), b"x").unwrap();
        assert!(!has_backup_on(&dir, day, "startup"), "a push backup doesn't count");
        std::fs::write(dir.join(backup_file_name(at("2026-09-28 23:59:59"), "startup")), b"x").unwrap();
        assert!(!has_backup_on(&dir, day, "startup"), "yesterday doesn't count");
        std::fs::write(dir.join(backup_file_name(at("2026-09-29 00:00:01"), "startup")), b"x").unwrap();
        assert!(has_backup_on(&dir, day, "startup"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_backup_leaves_no_partial_and_reports() {
        let dir = tmpdir("fail");
        let conn = Connection::open(dir.join("scheduler.duckdb")).unwrap();
        crate::migrations::run(&conn).unwrap();
        // A regular file where the backups dir should be → create_dir_all fails.
        let blocker = dir.join("backups");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let state = BackupState::default();
        let r = run(&conn, &dir.join("scheduler.duckdb"), "prepush", Some(&state));
        assert!(r.is_err());
        assert!(state.0.lock().unwrap().as_deref().unwrap().contains("prepush backup failed"));
        // Live connection unaffected.
        let n: i64 = conn.query_row("SELECT count(*) FROM _migrations", [], |r| r.get(0)).unwrap();
        assert!(n > 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
