//! Routine rotating backups of the live database.
//!
//! Files land in `<app_local_data>/backups/scheduler-YYYYMMDD-HHMMSS-<reason>.duckdb`
//! (next to scheduler.duckdb). Taken on startup (at most once per calendar
//! day), before every Sling push, and on demand from Settings. The newest
//! KEEP backups are kept; rotation only ever deletes files matching that
//! exact name pattern — nothing else in the folder is touched.
//!
//! The backup is an exact copy of scheduler.duckdb. On Windows the file can't
//! be copied while DuckDB holds it open (sharing violation, os error 32 — see
//! migrations::backup_if_pending), so under the Db mutex the live connection
//! is checkpointed, closed, the file copied, and the connection reopened
//! (see create_backup). The result is an ordinary DuckDB file that opens on
//! its own.
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

fn remove_partial(partial: &Path) {
    let _ = std::fs::remove_file(partial);
}

fn wal_path(db_file: &Path) -> PathBuf {
    let mut wal = db_file.as_os_str().to_owned();
    wal.push(".wal");
    PathBuf::from(wal)
}

/// Reopen the database file with the app's connection config. A couple of
/// retries: on Windows an antivirus scanner or the indexer can hold the file
/// for a moment right after our copy read it.
fn reopen(db_file: &Path) -> anyhow::Result<Connection> {
    let mut last = None;
    for attempt in 0..10 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
        }
        match crate::db::open_file(db_file) {
            Ok(c) => return Ok(c),
            Err(e) => last = Some(e),
        }
    }
    Err(anyhow::anyhow!(
        "could not reopen the database after the backup copy (restart Barrekeep): {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// While alive, the live connection has been closed and an in-memory
/// placeholder sits in its slot. `restore` (or, as a last resort, Drop —
/// e.g. on a panic) reopens the real file and swaps it back in.
struct Released<'a> {
    slot: &'a mut Connection,
    db_file: &'a Path,
    attempted: bool,
}

impl Released<'_> {
    fn restore(&mut self) -> anyhow::Result<()> {
        self.attempted = true;
        *self.slot = reopen(self.db_file)?;
        Ok(())
    }
}

impl Drop for Released<'_> {
    fn drop(&mut self) {
        if self.attempted {
            return;
        }
        if let Err(e) = self.restore() {
            crate::logging::write_line("backup", &format!("{e:#}"));
        }
    }
}

/// Checkpoint and close `conn` (the connection to `db_file`), leaving an
/// in-memory placeholder in its place so the file handle is released — on
/// Windows a file can't be copied while DuckDB holds it open (os error 32,
/// see migrations::backup_if_pending). The caller MUST hold the only
/// connection to the file (the app has exactly one: the `Db` mutex).
fn release<'a>(conn: &'a mut Connection, db_file: &'a Path) -> anyhow::Result<Released<'a>> {
    // Fold the WAL into the main file so the file alone is the whole DB.
    conn.execute_batch("CHECKPOINT;")?;
    let placeholder = crate::db::open_in_memory()?;
    let live = std::mem::replace(conn, placeholder);
    if let Err((live, e)) = live.close() {
        // Still open: put it back untouched.
        *conn = live;
        return Err(e.into());
    }
    Ok(Released { slot: conn, db_file, attempted: false })
}

/// Copy the database file into a new backup file in `dir`.
///
/// `conn` must be the (only) connection to `db_file`. It is checkpointed and
/// closed for the duration of the file copy, then reopened — always, even
/// when the copy fails — so the caller gets its working connection back.
/// Only if the reopen itself fails (after retries) is the slot left holding
/// an empty in-memory placeholder; the error then says to restart the app.
///
/// An exact file copy rather than `ATTACH … ; COPY FROM DATABASE`: COPY
/// FROM DATABASE inserts tables in an order that puts child rows before
/// their parents (availability_blocks before teachers), so on any DB with
/// FK-referencing rows it fails with "Violates foreign key constraint".
/// (EXPORT/IMPORT DATABASE has the same ordering hazard.) A file copy also
/// preserves sequences and every catalog detail byte for byte.
///
/// Writes to a `.partial` name and renames on success, so an interrupted
/// backup never shows up as a real one.
pub fn create_backup(
    conn: &mut Connection,
    db_file: &Path,
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

    let mut released = release(conn, db_file)?;
    let copied = (|| -> anyhow::Result<()> {
        // CHECKPOINT + a clean close leave no WAL. If one is somehow still
        // there, the main file alone would be missing its contents — refuse
        // rather than write a backup that silently lacks recent changes.
        if wal_path(db_file).exists() {
            anyhow::bail!(
                "{} still exists after CHECKPOINT; not taking a file-only backup",
                wal_path(db_file).display()
            );
        }
        std::fs::copy(db_file, &partial)?;
        std::fs::rename(&partial, &final_path)?;
        Ok(())
    })();
    let reopened = released.restore();
    drop(released);
    if let Err(e) = copied {
        remove_partial(&partial);
        return Err(match reopened {
            Ok(()) => e,
            Err(r) => e.context(format!("{r:#}")),
        });
    }
    reopened?;

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
    conn: &mut Connection,
    db_file: &Path,
    reason: &str,
    state: Option<&BackupState>,
) -> Result<BackupEntry, String> {
    let dir = backups_dir(db_file);
    let result = create_backup(conn, db_file, &dir, reason, chrono::Local::now().naive_local());
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
pub fn run_startup(conn: &mut Connection, db_file: &Path, state: &BackupState) {
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

// async: the file copy can take seconds; keep it off the UI thread
// (Tauri 2 runs plain sync commands on the main thread).
#[tauri::command(async)]
pub fn backup_now(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    state: State<'_, BackupState>,
) -> Result<BackupEntry, String> {
    let db_file = crate::db::db_path(&app).map_err(|e| e.to_string())?;
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    run(&mut conn, &db_file, "manual", Some(&state))
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

    /// Every base table with its row count, for comparing a backup with its
    /// source.
    fn table_counts(c: &Connection) -> Vec<(String, i64)> {
        let names: Vec<String> = c
            .prepare(
                "SELECT table_name FROM duckdb_tables()
                 WHERE database_name = current_database() AND NOT temporary
                 ORDER BY schema_name, table_name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        names
            .into_iter()
            .map(|t| {
                let n: i64 = c
                    .query_row(&format!("SELECT count(*) FROM \"{t}\""), [], |r| r.get(0))
                    .unwrap();
                (t, n)
            })
            .collect()
    }

    fn open_read_only(path: &Path) -> Connection {
        let cfg = duckdb::Config::default()
            .enable_autoload_extension(false)
            .unwrap()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap();
        Connection::open_with_flags(path, cfg).unwrap()
    }

    /// A migrated file DB populated like a real one: every FK edge
    /// (availability_blocks / teacher_qualifications / proposal_shifts →
    /// teachers, proposal_shifts / pushes → proposals, push_results →
    /// pushes) has referencing rows.
    fn populated_db(dir: &Path) -> (PathBuf, Connection) {
        let db_file = dir.join("scheduler.duckdb");
        let conn = crate::db::open_file(&db_file).unwrap();
        crate::migrations::run(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max)
             VALUES (29578230, 'Teacher A', 4, 5), (29578231, 'Teacher B', 3, 4);
             INSERT INTO positions (sling_position_id, class_name)
             VALUES (29470407, 'Classic'), (29470408, 'Empower');
             INSERT INTO teacher_qualifications (sling_user_id, sling_position_id)
             VALUES (29578230, 29470407), (29578230, 29470408), (29578231, 29470407);
             INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at)
             VALUES (29578230, 'availability', TIMESTAMPTZ '2026-11-02 08:00:00-06:00',
                                               TIMESTAMPTZ '2026-11-02 12:00:00-06:00'),
                    (29578231, 'leave',        TIMESTAMPTZ '2026-11-03 00:00:00-06:00',
                                               TIMESTAMPTZ '2026-11-04 00:00:00-06:00');
             INSERT INTO proposals (target_month, algorithm_version, parameters, is_current)
             VALUES ('2026-11', 'v9', '{}', TRUE);
             INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time,
                 sling_position_id, sling_user_id, generation_reason)
             SELECT id, DATE '2026-11-02', '09:00', '10:00', 29470407, 29578230, 'rotation'
             FROM proposals
             UNION ALL
             SELECT id, DATE '2026-11-03', '17:30', '18:15', 29470408, 29578231, 'rotation'
             FROM proposals;
             INSERT INTO pushes (proposal_id, shifts_attempted) SELECT id, 2 FROM proposals;
             INSERT INTO push_results (push_id, proposal_shift_id, outcome, sling_shift_id)
             SELECT p.id, s.id, 'created', CAST(s.id + 1000 AS VARCHAR)
             FROM pushes p, proposal_shifts s;",
        )
        .unwrap();
        (db_file, conn)
    }

    /// Regression: `COPY FROM DATABASE` copied child tables before their
    /// parents, so every backup of a populated DB failed with "Violates
    /// foreign key constraint because key sling_user_id: … does not exist
    /// in the referenced table".
    #[test]
    fn backup_of_db_with_fk_rows_matches_source() {
        let dir = tmpdir("fk");
        let (db_file, mut conn) = populated_db(&dir);
        let before = table_counts(&conn);
        for t in ["teachers", "availability_blocks", "teacher_qualifications", "proposal_shifts", "pushes", "push_results"] {
            assert!(before.iter().any(|(n, c)| n == t && *c > 0), "{t} must have rows");
        }
        let version = crate::migrations::current_version(&conn).unwrap();

        let entry = create_backup(&mut conn, &db_file, &backups_dir(&db_file), "manual", at("2026-09-29 10:00:00"))
            .expect("backup of a DB with FK-referencing rows");
        drop(conn);

        let b = open_read_only(Path::new(&entry.path));
        assert_eq!(table_counts(&b), before);
        assert_eq!(crate::migrations::current_version(&b).unwrap(), version);
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The live connection keeps working — and still writes to the real
    /// file, not the in-memory placeholder — after a backup succeeds and
    /// after the copy fails.
    #[test]
    fn live_connection_survives_success_and_copy_failure() {
        let dir = tmpdir("live");
        let (db_file, mut conn) = populated_db(&dir);
        let bdir = backups_dir(&db_file);
        let t = at("2026-09-29 11:00:00");
        let insert = "INSERT INTO proposals (target_month, algorithm_version, parameters) VALUES ('2026-12', 'v9', '{}')";

        create_backup(&mut conn, &db_file, &bdir, "manual", t).unwrap();
        conn.execute(insert, []).unwrap();

        // Force the copy itself to fail (after the connection was closed):
        // a directory squats on the next backup's .partial name.
        let squat = bdir.join(format!("{}{PARTIAL}", backup_file_name(t, "manual2")));
        std::fs::create_dir_all(&squat).unwrap();
        let err = create_backup(&mut conn, &db_file, &bdir, "manual", t).unwrap_err();
        assert!(!format!("{err:#}").contains("reopen"), "{err:#}");
        assert_eq!(list(&bdir).len(), 1, "a failed backup must not be listed");
        conn.execute(insert, []).unwrap();
        let n: i64 = conn.query_row("SELECT count(*) FROM proposals", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        drop(conn);

        // Both writes reached the file.
        let c = open_read_only(&db_file);
        let n: i64 = c.query_row("SELECT count(*) FROM proposals", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_of_live_db_reopens_with_data_and_sequences() {
        let dir = tmpdir("copy");
        let db_file = dir.join("scheduler.duckdb");
        let mut conn = crate::db::open_file(&db_file).unwrap();
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
        let entry = create_backup(&mut conn, &db_file, &bdir, "manual", at("2026-09-29 10:00:00")).unwrap();
        assert_eq!(entry.name, "scheduler-20260929-100000-manual.duckdb");
        assert!(entry.size_bytes > 0);
        // Live connection is still usable.
        let n: i64 = conn.query_row("SELECT count(*) FROM proposals", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        // Only the finished backup is left behind: no .partial, no .wal.
        let files: Vec<String> = std::fs::read_dir(&bdir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(files, vec![entry.name.clone()]);
        drop(conn);

        // The backup opens on its own and has the data + schema version.
        let b = crate::db::open_file(&entry.path).unwrap();
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
        let db_file = dir.join("scheduler.duckdb");
        let mut conn = crate::db::open_file(&db_file).unwrap();
        crate::migrations::run(&conn).unwrap();
        let bdir = dir.join("backups");
        let t = at("2026-09-29 10:00:00");
        let a = create_backup(&mut conn, &db_file, &bdir, "manual", t).unwrap();
        let b = create_backup(&mut conn, &db_file, &bdir, "manual", t).unwrap();
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
        let mut conn = crate::db::open_file(dir.join("scheduler.duckdb")).unwrap();
        crate::migrations::run(&conn).unwrap();
        // A regular file where the backups dir should be → create_dir_all fails.
        let blocker = dir.join("backups");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let state = BackupState::default();
        let r = run(&mut conn, &dir.join("scheduler.duckdb"), "prepush", Some(&state));
        assert!(r.is_err());
        assert!(state.0.lock().unwrap().as_deref().unwrap().contains("prepush backup failed"));
        // Live connection unaffected.
        let n: i64 = conn.query_row("SELECT count(*) FROM _migrations", [], |r| r.get(0)).unwrap();
        assert!(n > 0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
