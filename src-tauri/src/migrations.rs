// Forward-only migrations. Each entry is (version, label, sql). Versions must
// be unique and monotonically increasing. Once a migration is shipped, never
// edit its SQL — write a new migration instead.
//
// See .claude/skills/schema-change/ for the workflow when adding migrations.

use duckdb::Connection;

pub struct Migration {
    pub version: i32,
    pub label: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        label: "core schema",
        sql: include_str!("../migrations/0001_core_schema.sql"),
    },
    Migration {
        version: 2,
        label: "coteach_label on proposal_shifts",
        sql: include_str!("../migrations/0002_coteach_label.sql"),
    },
    Migration {
        version: 3,
        label: "drop FKs that reference proposal_shifts (DuckDB UPDATE limitation)",
        sql: include_str!("../migrations/0003_drop_proposal_shift_fks.sql"),
    },
    Migration {
        version: 4,
        label: "rebuild claude_runs without FKs",
        sql: include_str!("../migrations/0004_claude_runs_drop_fks.sql"),
    },
    Migration {
        version: 5,
        label: "sling pull: month_pulls + external_sling_shifts",
        sql: include_str!("../migrations/0005_sling_pull.sql"),
    },
    Migration {
        version: 6,
        label: "teacher location + sling_candidates",
        sql: include_str!("../migrations/0006_teacher_location.sql"),
    },
    Migration {
        version: 7,
        label: "studio_config singleton (runtime Sling ids)",
        sql: include_str!("../migrations/0007_studio_config.sql"),
    },
    Migration {
        version: 8,
        label: "purge demo roster + drop sling_candidates",
        sql: include_str!("../migrations/0008_drop_demo_roster.sql"),
    },
    Migration {
        version: 9,
        label: "make positions updatable: drop FKs into positions + UNIQUE(class_name)",
        sql: include_str!("../migrations/0009_positions_updatable.sql"),
    },
    Migration {
        version: 10,
        label: "claude editor: app_settings + algorithm_versions",
        sql: include_str!("../migrations/0010_algorithm_versions.sql"),
    },
    Migration {
        version: 11,
        label: "algorithm_versions.baseline_sha256",
        sql: include_str!("../migrations/0011_algorithm_baseline_sha.sql"),
    },
    Migration {
        version: 12,
        label: "multi-draft: proposal_drafts, month_push_candidate, claude_run_targets",
        sql: include_str!("../migrations/0012_multi_draft.sql"),
    },
    Migration {
        version: 13,
        label: "sling sync: push_result_snapshots + draft_checks",
        sql: include_str!("../migrations/0013_sling_sync.sql"),
    },
    Migration {
        version: 14,
        label: "availability sets + studio_hours + teacher_availability_windows",
        sql: include_str!("../migrations/0014_availability_sets.sql"),
    },
];

/// Run any migrations that haven't been applied yet. Idempotent.
pub fn run(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            label   VARCHAR NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
         );",
    )?;

    let applied: Vec<i32> = {
        let mut stmt = conn.prepare("SELECT version FROM _migrations ORDER BY version")?;
        let rows = stmt.query_map([], |row| row.get::<_, i32>(0))?;
        rows.collect::<Result<_, _>>()?
    };

    let mut applied_any = false;
    for m in MIGRATIONS {
        if applied.contains(&m.version) {
            continue;
        }
        crate::logging::write_line("migration", &format!("applying {} — {}", m.version, m.label));
        conn.execute_batch(m.sql)?;
        conn.execute(
            "INSERT INTO _migrations (version, label) VALUES (?, ?)",
            duckdb::params![m.version, m.label],
        )?;
        applied_any = true;
    }

    // Checkpoint after migrations so the WAL doesn't carry schema changes
    // across runs — a binary that dies mid-write shouldn't leave a WAL
    // referencing tables/columns the new binary might not have.
    if applied_any {
        let _ = conn.execute("CHECKPOINT", []);
    }

    Ok(())
}

/// If migrations are pending on an existing database, checkpoint and copy
/// the file aside first (scheduler.duckdb.backup-vN, N = schema version the
/// backup contains). Proposal/edit history exists nowhere but this file —
/// Sling can restore the roster, not the schedule history — so a botched
/// table-rebuild migration must be recoverable by hand. Fresh databases
/// (version 0) are skipped: nothing to lose yet. Returns the backup path
/// when one was made.
/// Opens its own short-lived connection and closes it BEFORE the copy:
/// Windows file locking is mandatory, so `fs::copy` on a database any handle
/// still has open fails with a sharing violation (os error 32) — that was
/// the v0.2.x instant-crash-at-startup on Windows machines with a pending
/// migration. Call before the long-lived `Db::open`.
pub fn backup_if_pending(
    db_file: &std::path::Path,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    if !db_file.exists() {
        return Ok(None);
    }
    let conn = crate::db::open_file(db_file)?;
    let current = current_version(&conn)?;
    // Flush any WAL replayed at open so the copy is a consistent snapshot.
    let _ = conn.execute("CHECKPOINT", []);
    conn.close().map_err(|(_, e)| e)?;
    let latest = MIGRATIONS.last().map(|m| m.version).unwrap_or(0);
    if current == 0 || current >= latest {
        return Ok(None);
    }
    let backup = db_file.with_extension(format!("duckdb.backup-v{current}"));
    std::fs::copy(db_file, &backup)?;
    Ok(Some(backup))
}

/// Highest applied migration version. 0 = unmigrated.
pub fn current_version(conn: &Connection) -> anyhow::Result<i32> {
    let v: Option<i32> = conn
        .query_row("SELECT max(version) FROM _migrations", [], |row| row.get(0))
        .ok()
        .flatten();
    Ok(v.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_db() -> Connection {
        let conn = crate::db::open_in_memory().expect("open");
        run(&conn).expect("migrations");
        // Roster + a generated proposal, i.e. the state where pull #2 used
        // to explode (positions referenced by quals and proposal_shifts).
        conn.execute_batch(
            "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max)
             VALUES (1930001, 'Alex Braun', 4, 5), (1930002, 'Kayla Moore', 4, 5);
             INSERT INTO positions (sling_position_id, class_name)
             VALUES (29470407, 'Classic'), (29470408, 'Empower');
             INSERT INTO teacher_qualifications (sling_user_id, sling_position_id)
             VALUES (1930001, 29470407), (1930002, 29470407);
             INSERT INTO proposals (target_month, algorithm_version, parameters, is_current)
             VALUES ('2026-08', 'v3', '{}', TRUE);
             INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time,
                 sling_position_id, sling_user_id, generation_reason)
             SELECT id, DATE '2026-08-03', '09:00', '10:00', 29470407, 1930001, 'rotation'
             FROM proposals;",
        )
        .expect("seed");
        conn
    }

    /// Regression test for the pull failure: sync_roster's unconditional
    /// class-name upsert must work while quals + proposal_shifts reference
    /// the position. Before migration 0009, UNIQUE(class_name) made the
    /// UPDATE an indexed rewrite, tripping the incoming FKs with
    /// "still referenced by a foreign key in a different table".
    #[test]
    fn positions_updatable_while_referenced() {
        let conn = fresh_db();
        conn.execute(
            "UPDATE positions SET class_name = 'Classic' WHERE sling_position_id = 29470407",
            [],
        )
        .expect("same-name update (every pull)");
        conn.execute(
            "UPDATE positions SET class_name = 'Classique' WHERE sling_position_id = 29470407",
            [],
        )
        .expect("rename update");
        conn.execute(
            "UPDATE positions SET active = FALSE WHERE sling_position_id = 29470408",
            [],
        )
        .expect("deactivate update");
    }

    /// The edit-teacher flow (migration 0003's original bug) must keep
    /// working on the tables rebuilt by 0009.
    #[test]
    fn edit_teacher_flow_still_works() {
        let mut conn = fresh_db();
        let tx = conn.transaction().expect("tx");
        tx.execute(
            "INSERT INTO edits (proposal_shift_id, field, old_value, new_value)
             SELECT id, 'sling_user_id', '1930001', '1930002' FROM proposal_shifts LIMIT 1",
            [],
        )
        .expect("edit row");
        tx.execute(
            "UPDATE proposal_shifts SET sling_user_id = 1930002, is_dropped = FALSE
             WHERE id = (SELECT min(id) FROM proposal_shifts)",
            [],
        )
        .expect("teacher swap");
        tx.commit().expect("commit");
    }

    /// Migration 0010 tables exist with the append-only/upsert shapes the
    /// commands rely on.
    #[test]
    fn migration_0010_tables() {
        let conn = fresh_db();
        conn.execute(
            "INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-opus-4-8')",
            [],
        ).expect("app_settings upsert");
        conn.execute(
            "INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-haiku-4-5')",
            [],
        ).expect("app_settings re-upsert");
        let v: String = conn.query_row(
            "SELECT value FROM app_settings WHERE key = 'claude_model'", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "claude-haiku-4-5");

        conn.execute(
            "INSERT INTO algorithm_versions (version, description, rules, created_by)
             VALUES (10, 'v10 — test', '{}', 'user')",
            [],
        ).expect("insert version row");
        let (ver, script): (i32, Option<String>) = conn.query_row(
            "SELECT version, script_file FROM algorithm_versions ORDER BY version DESC LIMIT 1",
            [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(ver, 10);
        assert!(script.is_none());
    }

    /// Migration 0011 adds baseline_sha256 as a nullable column.
    #[test]
    fn migration_0011_baseline_sha() {
        let conn = fresh_db();
        conn.execute(
            "INSERT INTO algorithm_versions (version, description, rules, created_by, baseline_sha256)
             VALUES (10, 'v10', '{}', 'user', 'abc'), (11, 'v11', '{}', 'user', NULL)",
            [],
        )
        .expect("insert with baseline_sha256");
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM algorithm_versions WHERE baseline_sha256 IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    /// Migration 0012 backfill: every pre-existing proposal gets a draft row
    /// ("Draft N" per month in creation order), every month a push draft
    /// (pushed > is_current > newest), every Claude run a target row — and
    /// re-running the SQL (e.g. a crash between the batch and the
    /// _migrations insert) changes nothing.
    #[test]
    fn migration_0012_backfill_is_idempotent() {
        let conn = crate::db::open_in_memory().expect("open");
        // Migrate up to 0011, seed history, then apply 0012.
        conn.execute_batch(
            "CREATE TABLE _migrations (version INTEGER PRIMARY KEY, label VARCHAR NOT NULL,
                 applied_at TIMESTAMPTZ NOT NULL DEFAULT now());",
        )
        .unwrap();
        for m in MIGRATIONS.iter().filter(|m| m.version < 12) {
            conn.execute_batch(m.sql).unwrap();
            conn.execute(
                "INSERT INTO _migrations (version, label) VALUES (?, ?)",
                duckdb::params![m.version, m.label],
            )
            .unwrap();
        }
        conn.execute_batch(
            "INSERT INTO proposals (id, target_month, algorithm_version, parameters, generated_at, is_current) VALUES
               (1, '2026-07', 'v9', '{}', TIMESTAMPTZ '2026-06-20 10:00:00+00', FALSE),
               (2, '2026-07', 'v9', '{}', TIMESTAMPTZ '2026-06-21 10:00:00+00', TRUE),
               (3, '2026-08', 'v9', '{}', TIMESTAMPTZ '2026-07-20 10:00:00+00', FALSE),
               (4, '2026-08', 'v9', '{}', TIMESTAMPTZ '2026-07-21 10:00:00+00', TRUE),
               (5, '2026-09', 'v9', '{}', TIMESTAMPTZ '2026-08-21 10:00:00+00', FALSE);
             -- August's older draft was the one pushed: it wins over is_current.
             INSERT INTO pushes (proposal_id) VALUES (3);
             INSERT INTO claude_runs (proposal_id, model, input_tokens, output_tokens,
                 input_text, output_text, cost_usd, duration_ms)
               VALUES (2, 'm', 1, 1, 'i', 'o', 0.01, 5), (NULL, 'm', 1, 1, 'i', 'o', 0.01, 5);",
        )
        .unwrap();
        run(&conn).expect("apply 0012");

        // (drafts, push candidates, claude_run_targets count)
        type Snapshot = (Vec<(i64, String)>, Vec<(String, i64)>, i64);
        let snapshot = |c: &Connection| -> Snapshot {
            let drafts = c
                .prepare("SELECT proposal_id, name FROM proposal_drafts ORDER BY proposal_id")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let cands = c
                .prepare("SELECT target_month, proposal_id FROM month_push_candidate ORDER BY 1")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let targets: i64 = c
                .query_row("SELECT count(*) FROM claude_run_targets", [], |r| r.get(0))
                .unwrap();
            (drafts, cands, targets)
        };
        let first = snapshot(&conn);
        assert_eq!(
            first.0,
            vec![
                (1, "Draft 1".to_string()),
                (2, "Draft 2".to_string()),
                (3, "Draft 1".to_string()),
                (4, "Draft 2".to_string()),
                (5, "Draft 1".to_string()),
            ]
        );
        assert_eq!(
            first.1,
            vec![("2026-07".to_string(), 2), ("2026-08".to_string(), 3), ("2026-09".to_string(), 5)]
        );
        assert_eq!(first.2, 1);

        // Re-running the migration SQL and the runner is a no-op.
        conn.execute_batch(MIGRATIONS.iter().find(|m| m.version == 12).unwrap().sql)
            .expect("0012 re-run");
        run(&conn).expect("runner re-run");
        assert_eq!(snapshot(&conn), first);

        // A user rename survives a re-run (never renumbered).
        conn.execute("UPDATE proposal_drafts SET name = 'Consistent days' WHERE proposal_id = 4", [])
            .unwrap();
        conn.execute_batch(MIGRATIONS.iter().find(|m| m.version == 12).unwrap().sql).unwrap();
        let name: String = conn
            .query_row("SELECT name FROM proposal_drafts WHERE proposal_id = 4", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Consistent days");
    }

    /// Migration 0013 is additive and re-runnable; its tables take the
    /// insert / INSERT OR REPLACE shapes push_sync and conflicts rely on.
    #[test]
    fn migration_0013_is_idempotent() {
        let conn = fresh_db();
        let sql = MIGRATIONS.iter().find(|m| m.version == 13).unwrap().sql;
        conn.execute_batch(
            "INSERT INTO pushes (id, proposal_id) SELECT 1, id FROM proposals;
             INSERT INTO push_results (push_id, proposal_shift_id, outcome, sling_shift_id)
               SELECT 1, id, 'created', '555' FROM proposal_shifts;
             INSERT INTO push_result_snapshots
               (push_result_id, sling_user_id, sling_position_id, shift_date, start_time, end_time)
               SELECT id, 1930001, 29470407, '2026-08-03', '09:00', '10:00' FROM push_results;
             INSERT OR REPLACE INTO draft_checks (proposal_id) SELECT id FROM proposals;
             INSERT OR REPLACE INTO draft_checks (proposal_id) SELECT id FROM proposals;",
        )
        .expect("0013 shapes");
        conn.execute_batch(sql).expect("0013 re-run");
        run(&conn).expect("runner re-run");
        let (snaps, checks): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM push_result_snapshots), (SELECT count(*) FROM draft_checks)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((snaps, checks), (1, 1));
        assert_eq!(current_version(&conn).unwrap(), MIGRATIONS.last().unwrap().version);
    }

    /// backup_if_pending: no-op when absent, fresh, or up to date; copies the
    /// file when a real database has pending migrations. On Windows (CI runs
    /// this on windows-latest) this also guards the v0.2.x startup crash:
    /// mandatory file locking means the copy only works because the function
    /// closes its own connection first — copying while any handle was open
    /// failed with a sharing violation (os error 32).
    #[test]
    fn backup_only_when_pending_on_existing_db() {
        let dir = std::env::temp_dir().join(format!("bk-mig-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_file = dir.join("scheduler.duckdb");
        let _ = std::fs::remove_file(&db_file);

        // No file at all -> skipped.
        assert!(backup_if_pending(&db_file).unwrap().is_none());

        // Fresh db, everything pending -> skipped (version 0, nothing to lose).
        {
            let _conn = crate::db::open_file(&db_file).expect("create file db");
        }
        assert!(backup_if_pending(&db_file).unwrap().is_none());

        // Fully migrated -> no backup.
        {
            let conn = crate::db::open_file(&db_file).expect("open file db");
            run(&conn).expect("migrations");
        }
        assert!(backup_if_pending(&db_file).unwrap().is_none());

        // Simulate an older install: pretend the last migration is pending.
        let latest = MIGRATIONS.last().unwrap().version;
        {
            let conn = crate::db::open_file(&db_file).expect("reopen");
            conn.execute("DELETE FROM _migrations WHERE version = ?", duckdb::params![latest])
                .unwrap();
        }
        let backup = backup_if_pending(&db_file).unwrap().expect("backup made");
        assert!(backup.exists());
        assert!(backup.to_string_lossy().ends_with(&format!("backup-v{}", latest - 1)));

        // The snapshot must itself be an openable database at the old version.
        let snap = crate::db::open_file(&backup).expect("backup opens");
        let v: i32 = snap
            .query_row("SELECT max(version) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, latest - 1);
        drop(snap);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Migration 0014: the three new tables exist with the shapes the pull
    /// relies on, and re-running the SQL (a crash between the batch and the
    /// _migrations insert) changes nothing and loses nothing.
    #[test]
    fn migration_0014_is_idempotent() {
        let conn = fresh_db();
        conn.execute_batch(
            "INSERT INTO sling_availability_sets (sling_set_id, sling_user_id, name, interval_raw, interval_days, raw_json)
               VALUES ('9001', 1930001, 'Mornings', '\"P1W\"', 7, '{}'),
                      (NULL, 1930001, NULL, NULL, NULL, '{}');
             INSERT INTO studio_hours (weekday, closed, open_time, close_time)
               VALUES (0, FALSE, '05:30', '19:30'), (6, TRUE, NULL, NULL);
             INSERT INTO teacher_availability_windows (target_month, sling_user_id, window_date, start_time, end_time)
               VALUES ('2026-11', 1930001, '2026-11-02', '05:30', '19:30');
             INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at) VALUES
               (1930001, 'availability_set', TIMESTAMPTZ '2026-11-03 09:45:00-06', TIMESTAMPTZ '2026-11-03 10:45:00-06'),
               (1930001, 'availability_set_pending', TIMESTAMPTZ '2026-11-04 09:45:00-06', TIMESTAMPTZ '2026-11-04 10:45:00-06');",
        )
        .expect("insert into 0014 tables");

        let counts = |c: &Connection| -> (i64, i64, i64, i64) {
            let n = |t: &str| c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)).unwrap();
            (n("sling_availability_sets"), n("studio_hours"), n("teacher_availability_windows"), n("availability_blocks"))
        };
        let before = counts(&conn);
        assert_eq!(before, (2, 2, 1, 2));
        let (pending, n): (bool, i32) = conn
            .query_row(
                "SELECT pending, availability_count FROM sling_availability_sets WHERE sling_set_id = '9001'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((pending, n), (false, 0), "defaults");

        let sql = MIGRATIONS.iter().find(|m| m.version == 14).expect("0014 registered").sql;
        conn.execute_batch(sql).expect("0014 re-run");
        conn.execute_batch(sql).expect("0014 re-run twice");
        assert_eq!(counts(&conn), before);
        run(&conn).expect("run is a no-op once applied");
        assert_eq!(current_version(&conn).unwrap(), MIGRATIONS.last().unwrap().version);

        // Sets and windows are replaced by DELETE + INSERT while teachers
        // reference nothing in them (no FKs in or out).
        conn.execute_batch(
            "DELETE FROM sling_availability_sets WHERE sling_user_id = 1930001;
             DELETE FROM teacher_availability_windows WHERE target_month = '2026-11';
             DELETE FROM studio_hours;
             INSERT INTO studio_hours (weekday, closed) VALUES (0, TRUE);",
        )
        .expect("delete + insert");
        // New set ids keep counting up after the delete.
        conn.execute("INSERT INTO sling_availability_sets (sling_user_id, raw_json) VALUES (1930002, '{}')", [])
            .unwrap();
        let id: i64 = conn.query_row("SELECT id FROM sling_availability_sets", [], |r| r.get(0)).unwrap();
        assert_eq!(id, 3);
    }
}
