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
        eprintln!("[migration] applying {} — {}", m.version, m.label);
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
pub fn backup_if_pending(
    conn: &Connection,
    db_file: &std::path::Path,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let current = current_version(conn)?;
    let latest = MIGRATIONS.last().map(|m| m.version).unwrap_or(0);
    if current == 0 || current >= latest || !db_file.exists() {
        return Ok(None);
    }
    // Flush any WAL replayed at open so the copy is a consistent snapshot.
    let _ = conn.execute("CHECKPOINT", []);
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
        let conn = Connection::open_in_memory().expect("open");
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

    /// backup_if_pending: no-op when up to date or fresh; copies the file
    /// when a real database has pending migrations.
    #[test]
    fn backup_only_when_pending_on_existing_db() {
        let dir = std::env::temp_dir().join(format!("bk-mig-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_file = dir.join("scheduler.duckdb");
        let _ = std::fs::remove_file(&db_file);

        let conn = Connection::open(&db_file).expect("open file db");

        // Fresh db, everything pending -> skipped (version 0, nothing to lose).
        assert!(backup_if_pending(&conn, &db_file).unwrap().is_none());

        run(&conn).expect("migrations");
        // Fully migrated -> no backup.
        assert!(backup_if_pending(&conn, &db_file).unwrap().is_none());

        // Simulate an older install: pretend the last migration is pending.
        let latest = MIGRATIONS.last().unwrap().version;
        conn.execute("DELETE FROM _migrations WHERE version = ?", duckdb::params![latest]).unwrap();
        let backup = backup_if_pending(&conn, &db_file).unwrap().expect("backup made");
        assert!(backup.exists());
        assert!(backup.to_string_lossy().ends_with(&format!("backup-v{}", latest - 1)));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

