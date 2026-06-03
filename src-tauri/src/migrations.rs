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

/// Highest applied migration version. 0 = unmigrated.
pub fn current_version(conn: &Connection) -> anyhow::Result<i32> {
    let v: Option<i32> = conn
        .query_row("SELECT max(version) FROM _migrations", [], |row| row.get(0))
        .ok()
        .flatten();
    Ok(v.unwrap_or(0))
}
