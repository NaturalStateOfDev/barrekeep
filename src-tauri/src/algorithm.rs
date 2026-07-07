//! Versioned algorithm store (spec: docs/superpowers/specs/
//! 2026-07-06-claude-proposal-editor-design.md).
//!
//! One version sequence: v9 is the implicit baseline (the shipped
//! scripts/propose.py with empty rules); adopted versions start at 10 and
//! live as append-only rows in `algorithm_versions`. Each row carries a
//! FULL rules snapshot and optionally a script file under
//! `<app_local_data>/algorithms/` (NULL = baseline script). Rows are only
//! ever inserted — "last used" derives from proposals.algorithm_version.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const BASELINE_VERSION: i32 = 9;
const ARCHIVE_VERSIONS_BEHIND: i32 = 3;
const ARCHIVE_UNUSED_MONTHS: u32 = 3;

// ============================================================
// Rules schema (v1) — mirrors what scripts/propose.py consumes.
// deny_unknown_fields guards against prompt drift: Claude inventing rule
// keys fails validation instead of being silently stored and ignored.
// ============================================================

const WEEKDAYS: &[&str] = &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(default)]
    pub teacher_class_blocklist: Vec<ClassBlock>,
    #[serde(default)]
    pub teacher_slot_blocklist: Vec<SlotBlock>,
    #[serde(default)]
    pub priority_slots: Vec<PrioritySlot>,
    #[serde(default)]
    pub slot_class_overrides: Vec<SlotClassOverride>,
    #[serde(default)]
    pub variety_penalty_multiplier: std::collections::HashMap<String, f64>,
    #[serde(default)]
    pub variety_penalty_per_class: Option<f64>,
    #[serde(default)]
    pub sat_time_shifts: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub sun_time_shifts: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ClassBlock {
    pub sling_user_id: i32,
    pub class_name: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SlotBlock {
    pub sling_user_id: i32,
    pub weekday: String,
    pub time: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct PrioritySlot {
    pub sling_user_id: i32,
    pub weekday: String,
    pub time: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SlotClassOverride {
    pub weekday: String,
    pub time: String,
    pub class_name: String,
}

fn check_weekday(day: &str, ctx: &str) -> Result<(), String> {
    if WEEKDAYS.contains(&day) {
        Ok(())
    } else {
        Err(format!("{ctx}: unknown weekday '{day}' (use Mon..Sun)"))
    }
}

fn check_time(t: &str, ctx: &str) -> Result<(), String> {
    let ok = t.len() == 5
        && t.as_bytes()[2] == b':'
        && t[..2].parse::<u32>().map(|h| h < 24).unwrap_or(false)
        && t[3..].parse::<u32>().map(|m| m < 60).unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(format!("{ctx}: bad time '{t}' (use HH:MM)"))
    }
}

/// Parse + validate a raw rules JSON value into the typed schema. Errors on
/// unknown keys, unknown weekday names, and malformed times.
pub fn validate_rules(raw: &serde_json::Value) -> Result<Rules, String> {
    let rules: Rules = serde_json::from_value(raw.clone())
        .map_err(|e| format!("rules do not match the schema: {e}"))?;
    for b in &rules.teacher_slot_blocklist {
        check_weekday(&b.weekday, "teacher_slot_blocklist")?;
        check_time(&b.time, "teacher_slot_blocklist")?;
    }
    for p in &rules.priority_slots {
        check_weekday(&p.weekday, "priority_slots")?;
        check_time(&p.time, "priority_slots")?;
    }
    for o in &rules.slot_class_overrides {
        check_weekday(&o.weekday, "slot_class_overrides")?;
        check_time(&o.time, "slot_class_overrides")?;
    }
    for (uid, _) in &rules.variety_penalty_multiplier {
        uid.parse::<i32>()
            .map_err(|_| format!("variety_penalty_multiplier: key '{uid}' is not a teacher id"))?;
    }
    Ok(rules)
}

// ============================================================
// Version store
// ============================================================

#[derive(Debug, Serialize, Clone)]
pub struct AlgorithmVersion {
    pub version: i32,
    pub description: String,
    pub rules: serde_json::Value,
    pub script_file: Option<String>,
    pub created_by: String,
    pub adopted_at: String,
    pub last_used_month: Option<String>,
    pub script_archived: bool,
    pub script_missing: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn row_to_version(
    conn: &duckdb::Connection,
    version: i32,
    description: String,
    rules_text: String,
    script_file: Option<String>,
    created_by: String,
    adopted_at: String,
    algo_dir: Option<&Path>,
) -> AlgorithmVersion {
    let last_used_month: Option<String> = conn
        .query_row(
            "SELECT max(target_month) FROM proposals WHERE algorithm_version = ?",
            duckdb::params![format!("v{version}")],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let (script_archived, script_missing) = match (&script_file, algo_dir) {
        (Some(f), Some(dir)) => {
            let live = dir.join(f).exists();
            let archived = dir.join("archive").join(f).exists();
            (!live && archived, !live && !archived)
        }
        _ => (false, false),
    };
    AlgorithmVersion {
        version,
        description,
        rules: serde_json::from_str(&rules_text).unwrap_or(serde_json::Value::Null),
        script_file,
        created_by,
        adopted_at,
        last_used_month,
        script_archived,
        script_missing,
    }
}

/// Newest adopted version, or None when only the v9 baseline exists.
pub fn active_version(conn: &duckdb::Connection) -> Result<Option<AlgorithmVersion>, String> {
    let row = conn.query_row(
        "SELECT version, description, CAST(rules AS VARCHAR), script_file, created_by,
                CAST(adopted_at AS VARCHAR)
         FROM algorithm_versions ORDER BY version DESC LIMIT 1",
        [],
        |r| {
            Ok((
                r.get::<_, i32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        },
    );
    match row {
        Ok((v, d, rules, sf, cb, at)) => {
            Ok(Some(row_to_version(conn, v, d, rules, sf, cb, at, None)))
        }
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(err(e)),
    }
}

/// All adopted versions, newest first, with script file status.
pub fn list_versions(
    conn: &duckdb::Connection,
    algo_dir: &Path,
) -> Result<Vec<AlgorithmVersion>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT version, description, CAST(rules AS VARCHAR), script_file, created_by,
                    CAST(adopted_at AS VARCHAR)
             FROM algorithm_versions ORDER BY version DESC",
        )
        .map_err(err)?;
    let rows: Vec<(i32, String, String, Option<String>, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    Ok(rows
        .into_iter()
        .map(|(v, d, rules, sf, cb, at)| {
            row_to_version(conn, v, d, rules, sf, cb, at, Some(algo_dir))
        })
        .collect())
}

/// Adopt a new version: validate the rules, assign version = max + 1
/// (starting at 10), write the script file (if any) BEFORE inserting the
/// row, insert append-only. Returns the new version number.
pub fn adopt_version(
    conn: &duckdb::Connection,
    algo_dir: &Path,
    description: &str,
    rules_raw: &serde_json::Value,
    script_content: Option<&str>,
    claude_run_id: Option<i64>,
) -> Result<i32, String> {
    if description.trim().is_empty() {
        return Err("description is required".to_string());
    }
    validate_rules(rules_raw)?;

    let max_existing: Option<i32> = conn
        .query_row("SELECT max(version) FROM algorithm_versions", [], |r| {
            r.get(0)
        })
        .ok()
        .flatten();
    let version = max_existing.unwrap_or(BASELINE_VERSION).max(BASELINE_VERSION) + 1;

    let script_file = match script_content {
        Some(content) => {
            let name = format!("propose_v{version}.py");
            std::fs::create_dir_all(algo_dir).map_err(err)?;
            std::fs::write(algo_dir.join(&name), content).map_err(err)?;
            Some(name)
        }
        None => None,
    };

    let created_by = if claude_run_id.is_some() { "claude" } else { "user" };
    conn.execute(
        "INSERT INTO algorithm_versions (version, description, rules, script_file, created_by, claude_run_id)
         VALUES (?, ?, ?, ?, ?, ?)",
        duckdb::params![
            version,
            description,
            serde_json::to_string(rules_raw).map_err(err)?,
            script_file,
            created_by,
            claude_run_id
        ],
    )
    .map_err(err)?;
    Ok(version)
}

/// Resolve the script to run for a version. NULL script_file = the shipped
/// baseline; otherwise algorithms/{file}, falling back to archive/{file}.
pub fn resolve_script(
    algo_dir: &Path,
    version: &AlgorithmVersion,
    project_root: &Path,
) -> Result<PathBuf, String> {
    match &version.script_file {
        None => Ok(project_root.join("scripts").join("propose.py")),
        Some(f) => {
            let live = algo_dir.join(f);
            if live.exists() {
                return Ok(live);
            }
            let archived = algo_dir.join("archive").join(f);
            if archived.exists() {
                return Ok(archived);
            }
            Err(format!(
                "algorithm script {f} was deleted — adopt a newer version or re-adopt the rules on the baseline"
            ))
        }
    }
}

/// Startup sweep: move script files that are more than
/// ARCHIVE_VERSIONS_BEHIND versions behind the active one AND unused for
/// ARCHIVE_UNUSED_MONTHS months (or never used) into algorithms/archive/.
/// Returns the moved file names. Deletion stays manual-only.
pub fn archive_sweep(conn: &duckdb::Connection, algo_dir: &Path) -> Result<Vec<String>, String> {
    let versions = list_versions(conn, algo_dir)?;
    let Some(active) = versions.first().map(|v| v.version) else {
        return Ok(Vec::new());
    };
    let cutoff_month = {
        // "YYYY-MM" ARCHIVE_UNUSED_MONTHS ago, computed from the DB clock so
        // tests and app agree on "now".
        let now: String = conn
            .query_row("SELECT strftime(now(), '%Y-%m')", [], |r| r.get(0))
            .map_err(err)?;
        let (y, m): (i32, u32) = {
            let parts: Vec<&str> = now.split('-').collect();
            (parts[0].parse().map_err(err)?, parts[1].parse().map_err(err)?)
        };
        let mut y2 = y;
        let mut m2 = m as i32 - ARCHIVE_UNUSED_MONTHS as i32;
        while m2 < 1 {
            m2 += 12;
            y2 -= 1;
        }
        format!("{y2:04}-{m2:02}")
    };

    let mut moved = Vec::new();
    for v in &versions {
        let Some(file) = &v.script_file else { continue };
        if v.version >= active - ARCHIVE_VERSIONS_BEHIND {
            continue;
        }
        let unused = match &v.last_used_month {
            None => true,
            Some(m) => m.as_str() < cutoff_month.as_str(),
        };
        if !unused {
            continue;
        }
        let live = algo_dir.join(file);
        if !live.exists() {
            continue;
        }
        let archive_dir = algo_dir.join("archive");
        std::fs::create_dir_all(&archive_dir).map_err(err)?;
        std::fs::rename(&live, archive_dir.join(file)).map_err(err)?;
        moved.push(file.clone());
    }
    Ok(moved)
}

/// `<app_local_data>/algorithms`, created on demand.
pub fn algorithms_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("could not resolve app_local_data_dir: {e}"))?
        .join("algorithms");
    std::fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conn() -> duckdb::Connection {
        let c = duckdb::Connection::open_in_memory().expect("open");
        crate::migrations::run(&c).expect("migrations");
        c
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bk-algo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validate_rules_rejects_unknown_keys_and_bad_values() {
        assert!(validate_rules(&json!({})).is_ok());
        assert!(validate_rules(&json!({"hard_assignments": []})).is_err());
        assert!(validate_rules(&json!({
            "teacher_slot_blocklist": [{"sling_user_id": 1, "weekday": "Sax", "time": "08:00"}]
        }))
        .is_err());
        assert!(validate_rules(&json!({
            "teacher_slot_blocklist": [{"sling_user_id": 1, "weekday": "Sat", "time": "8am"}]
        }))
        .is_err());
        assert!(validate_rules(&json!({
            "variety_penalty_multiplier": {"not-a-uid": 2.0}
        }))
        .is_err());
        let ok = validate_rules(&json!({
            "teacher_class_blocklist": [{"sling_user_id": 501, "class_name": "Reform", "reason": "r"}],
            "priority_slots": [{"sling_user_id": 501, "weekday": "Mon", "time": "09:00"}],
            "variety_penalty_per_class": 0.5
        }))
        .unwrap();
        assert_eq!(ok.teacher_class_blocklist.len(), 1);
    }

    #[test]
    fn adopt_assigns_sequential_versions_and_writes_scripts() {
        let c = conn();
        let dir = scratch("adopt");
        let v1 = adopt_version(&c, &dir, "v10 — rules only", &json!({}), None, None).unwrap();
        assert_eq!(v1, 10);
        let v2 = adopt_version(
            &c,
            &dir,
            "v11 — code",
            &json!({}),
            Some("print('hi')"),
            Some(42),
        )
        .unwrap();
        assert_eq!(v2, 11);
        assert!(dir.join("propose_v11.py").exists());

        let versions = list_versions(&c, &dir).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 11);
        assert_eq!(versions[0].created_by, "claude");
        assert_eq!(versions[1].created_by, "user");
        assert!(active_version(&c).unwrap().unwrap().version == 11);

        // Invalid rules refuse adoption and burn no version number.
        assert!(adopt_version(&c, &dir, "bad", &json!({"nope": 1}), None, None).is_err());
        assert_eq!(active_version(&c).unwrap().unwrap().version, 11);
    }

    #[test]
    fn resolve_script_falls_back_to_archive_then_errors() {
        let c = conn();
        let dir = scratch("resolve");
        let root = scratch("resolve-root");
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::write(root.join("scripts/propose.py"), "baseline").unwrap();

        // Baseline (no rows): rules-only adopt resolves to the shipped script.
        let v = adopt_version(&c, &dir, "v10", &json!({}), None, None).unwrap();
        let versions = list_versions(&c, &dir).unwrap();
        let v10 = versions.iter().find(|x| x.version == v).unwrap();
        assert_eq!(
            resolve_script(&dir, v10, &root).unwrap(),
            root.join("scripts/propose.py")
        );

        // Script version: live → archive → error.
        adopt_version(&c, &dir, "v11", &json!({}), Some("code"), None).unwrap();
        let versions = list_versions(&c, &dir).unwrap();
        let v11 = versions.iter().find(|x| x.version == 11).unwrap();
        assert_eq!(resolve_script(&dir, v11, &root).unwrap(), dir.join("propose_v11.py"));

        std::fs::create_dir_all(dir.join("archive")).unwrap();
        std::fs::rename(dir.join("propose_v11.py"), dir.join("archive/propose_v11.py")).unwrap();
        assert_eq!(
            resolve_script(&dir, v11, &root).unwrap(),
            dir.join("archive/propose_v11.py")
        );

        std::fs::remove_file(dir.join("archive/propose_v11.py")).unwrap();
        let e = resolve_script(&dir, v11, &root).unwrap_err();
        assert!(e.contains("adopt a newer version"), "{e}");
    }

    #[test]
    fn archive_sweep_only_old_and_unused() {
        let c = conn();
        let dir = scratch("sweep");
        // v10..v15, all with script files; active = 15, so v10 and v11 are
        // more than 3 versions behind.
        for _ in 10..=15 {
            adopt_version(&c, &dir, "v", &json!({}), Some("code"), None).unwrap();
        }
        // v11 used recently (protected); v10 used long ago (sweepable);
        // v12..15 are within 3 of active regardless of use.
        c.execute_batch(
            "INSERT INTO proposals (target_month, algorithm_version, parameters, generated_at)
             VALUES (strftime(now(), '%Y-%m'), 'v11', '{}', now());
             INSERT INTO proposals (target_month, algorithm_version, parameters, generated_at)
             VALUES (strftime(CAST(now() AS TIMESTAMP) - INTERVAL 200 DAYS, '%Y-%m'), 'v10', '{}',
                     CAST(now() AS TIMESTAMP) - INTERVAL 200 DAYS);",
        )
        .unwrap();

        let moved = archive_sweep(&c, &dir).unwrap();
        assert_eq!(moved, vec!["propose_v10.py".to_string()]);
        assert!(dir.join("archive/propose_v10.py").exists());
        assert!(dir.join("propose_v11.py").exists(), "recently used stays");
        assert!(dir.join("propose_v12.py").exists(), "within 3 of active stays");

        // Idempotent: second sweep moves nothing.
        assert!(archive_sweep(&c, &dir).unwrap().is_empty());
    }
}
