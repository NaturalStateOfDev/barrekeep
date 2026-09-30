// Tauri IPC commands — the surface the React frontend calls into.
// Add new commands here, then register them in lib.rs's invoke_handler!.
//
// Convention: commands return Result<T, String> so errors serialize to JS
// as plain strings (anyhow's full chain via {:#}).

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

use crate::db::{db_path, Db};
use crate::migrations;
use crate::review;
use crate::sling;

/// Anthropic API key, held in memory only — paste it once per session via
/// the Settings tab. Stronghold-backed persistence comes later.
pub struct AnthropicKey(pub Mutex<Option<String>>);

/// Sling auth token. Loaded from Stronghold at app start; held in memory
/// for the session. Stronghold is the persistence layer — this Mutex is
/// the in-memory cache to avoid keychain reads on every request.
pub struct SlingToken(pub Mutex<Option<String>>);

/// Org id opportunistically parsed from the Sling login request URL, used as a
/// fallback when account/session doesn't expose it. See sling_login.rs.
pub struct SlingOrgHint(pub Mutex<Option<i64>>);

#[derive(Serialize)]
pub struct DbInfo {
    pub path: String,
    pub schema_version: i32,
    pub teacher_count: i64,
    pub position_count: i64,
}

#[derive(Serialize)]
pub struct Teacher {
    pub sling_user_id: i32,
    pub display_name: String,
    pub weekly_target: i32,
    pub weekly_max: i32,
    pub is_lead: bool,
    pub ranking_weight: f64,
    pub variety_multiplier: f64,
    pub active: bool,
    pub notes: Option<String>,
    pub locations: Option<String>,
}

#[derive(Serialize)]
pub struct Position {
    pub sling_position_id: i32,
    pub class_name: String,
    pub duration_minutes: i32,
    pub is_special: bool,
    pub active: bool,
}

#[derive(Serialize)]
pub struct PullResult {
    pub target_month: String,
    pub pulled_at: String,
    pub user_count: i64,
    pub qual_count: i64,
    pub availability_count: i64,
    pub external_shift_count: i64,
    pub history_shift_count: i64,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// Load the singleton studio_config row (migration 0007). Placeholder zeros
/// until the user configures their studio in Settings.
fn load_studio_config(conn: &duckdb::Connection) -> Result<sling::StudioConfig, String> {
    conn.query_row(
        "SELECT org_id, acting_user_id, home_location_id FROM studio_config WHERE id = 1",
        [],
        |r| {
            Ok(sling::StudioConfig {
                org_id: r.get(0)?,
                acting_user_id: r.get(1)?,
                home_location_id: r.get(2)?,
            })
        },
    )
    .map_err(err)
}

/// load_studio_config, erroring while the ids are still placeholders.
pub(crate) fn load_studio_config_checked(conn: &duckdb::Connection) -> Result<sling::StudioConfig, String> {
    let cfg = load_studio_config(conn)?;
    if cfg.org_id == 0 || cfg.home_location_id == 0 {
        return Err(
            "Studio not configured — use “Set up studio” (it detects your org and \
             location from your Sling login) or enter the IDs in Settings → Studio configuration first."
                .to_string(),
        );
    }
    Ok(cfg)
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioConfigDto {
    pub org_id: i64,
    pub acting_user_id: i64,
    pub home_location_id: i64,
}

#[tauri::command]
pub fn get_studio_config(db: State<'_, Db>) -> Result<StudioConfigDto, String> {
    let conn = db.0.lock().map_err(err)?;
    let c = load_studio_config(&conn)?;
    Ok(StudioConfigDto {
        org_id: c.org_id,
        acting_user_id: c.acting_user_id,
        home_location_id: c.home_location_id,
    })
}

#[tauri::command]
pub fn set_studio_config(
    db: State<'_, Db>,
    org_id: i64,
    acting_user_id: i64,
    home_location_id: i64,
) -> Result<(), String> {
    if org_id < 0 || acting_user_id < 0 || home_location_id < 0 {
        return Err("IDs must be non-negative".to_string());
    }
    let conn = db.0.lock().map_err(err)?;
    write_studio_config(&conn, org_id, acting_user_id, home_location_id)
}

/// Update the singleton studio_config row. Compare-before-write (see the
/// DuckDB UPDATE gotcha in CLAUDE.md): an unchanged row is not touched.
fn write_studio_config(
    conn: &duckdb::Connection,
    org_id: i64,
    acting_user_id: i64,
    home_location_id: i64,
) -> Result<(), String> {
    let cur = load_studio_config(conn)?;
    if cur.org_id == org_id
        && cur.acting_user_id == acting_user_id
        && cur.home_location_id == home_location_id
    {
        return Ok(());
    }
    conn.execute(
        "UPDATE studio_config
         SET org_id = ?, acting_user_id = ?, home_location_id = ?, updated_at = now()
         WHERE id = 1",
        duckdb::params![org_id, acting_user_id, home_location_id],
    )
    .map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn db_info(app: tauri::AppHandle, db: State<'_, Db>) -> Result<DbInfo, String> {
    let conn = db.0.lock().map_err(err)?;
    let schema_version = migrations::current_version(&conn).map_err(err)?;
    let teacher_count: i64 = conn
        .query_row("SELECT count(*) FROM teachers", [], |r| r.get(0))
        .map_err(err)?;
    let position_count: i64 = conn
        .query_row("SELECT count(*) FROM positions", [], |r| r.get(0))
        .map_err(err)?;
    let path = db_path(&app).map_err(err)?;
    Ok(DbInfo {
        path: path.display().to_string(),
        schema_version,
        teacher_count,
        position_count,
    })
}

#[tauri::command]
pub fn list_teachers(db: State<'_, Db>) -> Result<Vec<Teacher>, String> {
    let conn = db.0.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT sling_user_id, display_name, weekly_target, weekly_max,
                    is_lead, ranking_weight, variety_multiplier, active, notes, locations
             FROM teachers
             ORDER BY is_lead DESC, display_name",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Teacher {
                sling_user_id: r.get(0)?,
                display_name: r.get(1)?,
                weekly_target: r.get(2)?,
                weekly_max: r.get(3)?,
                is_lead: r.get(4)?,
                ranking_weight: r.get(5)?,
                variety_multiplier: r.get(6)?,
                active: r.get(7)?,
                notes: r.get(8)?,
                locations: r.get(9)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

#[tauri::command]
pub fn update_teacher_settings(
    db: State<'_, Db>,
    sling_user_id: i32,
    weekly_target: i32,
    weekly_max: i32,
) -> Result<(), String> {
    if weekly_target < 0 || weekly_max < 0 {
        return Err("target and max must be >= 0".to_string());
    }
    let conn = db.0.lock().map_err(err)?;
    let n = conn.execute(
        "UPDATE teachers SET weekly_target = ?, weekly_max = ? WHERE sling_user_id = ?",
        duckdb::params![weekly_target, weekly_max, sling_user_id],
    ).map_err(err)?;
    if n == 0 {
        return Err(format!("no teacher with sling_user_id={sling_user_id}"));
    }
    Ok(())
}

#[tauri::command]
pub fn list_positions(db: State<'_, Db>) -> Result<Vec<Position>, String> {
    let conn = db.0.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT sling_position_id, class_name, duration_minutes, is_special, active
             FROM positions
             ORDER BY is_special DESC, class_name",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Position {
                sling_position_id: r.get(0)?,
                class_name: r.get(1)?,
                duration_minutes: r.get(2)?,
                is_special: r.get(3)?,
                active: r.get(4)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

#[tauri::command]
pub fn set_position_active(db: State<'_, Db>, sling_position_id: i32, active: bool) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    conn.execute("UPDATE positions SET active = ? WHERE sling_position_id = ?",
        duckdb::params![active, sling_position_id]).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn list_qualified_pairs(db: State<'_, Db>) -> Result<Vec<String>, String> {
    let conn = db.0.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT sling_user_id, sling_position_id
             FROM teacher_qualifications
             WHERE NOT is_blocklisted",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| {
            let u: i32 = r.get(0)?;
            let p: i32 = r.get(1)?;
            Ok(format!("{}:{}", u, p))
        })
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

// ============================================================
// Proposal generation
// ============================================================

/// JSON payload emitted by `scripts/propose.py --json-out`. Must match the
/// shape produced at the bottom of propose.py.
#[derive(Deserialize)]
struct ProposeOutput {
    algorithm_version: String,
    target_month: String,
    parameters: serde_json::Value,
    shifts: Vec<ProposeShift>,
}

// class_name is only read by candidate validation (per-slot diff labels);
// stored proposals get class names from a JOIN on positions.
#[derive(Deserialize)]
struct ProposeShift {
    shift_date: String,
    start_time: String,
    end_time: String,
    #[serde(default)]
    class_name: String,
    sling_position_id: i32,
    sling_user_id: Option<i32>,
    generation_reason: String,
    flag: String,
    is_coteach: bool,
    coteach_label: String,
    is_dropped: bool,
}

#[derive(Serialize)]
pub struct GenerateResult {
    pub proposal_id: i64,
    pub target_month: String,
    pub algorithm_version: String,
    pub shift_count: usize,
    pub dropped_count: usize,
    pub stderr_tail: String,
}

/// Build the stdin payload propose.py expects, straight from the DB.
/// Shared by generate_proposal and the code-draft validator.
fn build_propose_payload(
    conn: &duckdb::Connection,
    target_month: &str,
) -> Result<serde_json::Value, String> {
    // Studio's home location id — propose.py filters shifts to this.
    let studio_cfg = load_studio_config(conn)?;
    let payload_json = {

        let teachers: Vec<serde_json::Value> = {
            let mut stmt = conn.prepare(
                "SELECT sling_user_id, display_name, weekly_target, weekly_max,
                        is_lead, ranking_weight, variety_multiplier, active
                 FROM teachers WHERE active = TRUE"
            ).map_err(err)?;
            stmt.query_map([], |r| Ok(serde_json::json!({
                "sling_user_id": r.get::<_, i32>(0)?,
                "display_name": r.get::<_, String>(1)?,
                "weekly_target": r.get::<_, i32>(2)?,
                "weekly_max": r.get::<_, i32>(3)?,
                "is_lead": r.get::<_, bool>(4)?,
                "ranking_weight": r.get::<_, f64>(5)?,
                "variety_multiplier": r.get::<_, f64>(6)?,
                "active": r.get::<_, bool>(7)?,
            }))).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
        };

        let users_with_groups: Vec<serde_json::Value> = {
            let mut stmt = conn.prepare(
                "SELECT t.sling_user_id, t.display_name,
                        list(tq.sling_position_id) FILTER (WHERE NOT tq.is_blocklisted)
                 FROM teachers t
                 LEFT JOIN teacher_qualifications tq ON tq.sling_user_id = t.sling_user_id
                 GROUP BY t.sling_user_id, t.display_name"
            ).map_err(err)?;
            stmt.query_map([], |r| {
                let uid: i32 = r.get(0)?;
                let name: String = r.get(1)?;
                // list() aggregate returns duckdb::types::Value::List; extract as i32 array.
                let raw: duckdb::types::Value = r.get(2)?;
                let group_ids: Vec<i32> = match raw {
                    duckdb::types::Value::List(items) => items.into_iter().filter_map(|v| {
                        match v {
                            duckdb::types::Value::Int(n) => Some(n),
                            duckdb::types::Value::SmallInt(n) => Some(n as i32),
                            duckdb::types::Value::BigInt(n) => Some(n as i32),
                            _ => None,
                        }
                    }).collect(),
                    _ => vec![],
                };
                Ok(serde_json::json!({
                    "id": uid,
                    "lastname": "",
                    "name": name,
                    "groupIds": group_ids,
                }))
            }).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
        };

        // History shifts: trailing 3 months for ranking weights.
        let history_events: Vec<serde_json::Value> = {
            let (y, m): (i32, u32) = {
                let p: Vec<&str> = target_month.split('-').collect();
                (p[0].parse().map_err(err)?, p[1].parse().map_err(err)?)
            };
            let mut y2 = y; let mut m2 = m as i32 - 3;
            while m2 < 1 { m2 += 12; y2 -= 1; }
            let cutoff = format!("{y2:04}-{m2:02}");
            let mut stmt = conn.prepare(
                "SELECT CAST(shift_date AS VARCHAR), start_time, end_time, sling_user_id, sling_position_id
                 FROM external_sling_shifts
                 WHERE target_month >= ? AND target_month < ?"
            ).map_err(err)?;
            stmt.query_map(duckdb::params![&cutoff, target_month], |r| {
                let date: String = r.get(0)?;
                let start: String = r.get(1)?;
                let end: String = r.get(2)?;
                let uid: Option<i32> = r.get(3)?;
                let pid: i32 = r.get(4)?;
                Ok(serde_json::json!({
                    "type": "shift",
                    "dtstart": crate::sling::shift_iso(&date, &start),
                    "dtend": crate::sling::shift_iso(&date, &end),
                    "user": uid.map(|u| serde_json::json!({"id": u})),
                    "position": {"id": pid},
                    "location": {"id": studio_cfg.home_location_id},
                }))
            }).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
        };

        // Month events: availability + leave (overlapping the month, see
        // query_availability_blocks) + existing shifts for the target month.
        let month_events: Vec<serde_json::Value> = {
            let mut events: Vec<serde_json::Value> = query_availability_blocks(&conn, &target_month)?
                .into_iter()
                .map(|b| serde_json::json!({
                    "type": b.source,
                    "dtstart": b.starts_at,
                    "dtend": b.ends_at,
                    "user": {"id": b.sling_user_id},
                }))
                .collect();
            // Append target-month external shifts
            let mut stmt2 = conn.prepare(
                "SELECT CAST(shift_date AS VARCHAR), start_time, end_time, sling_user_id, sling_position_id
                 FROM external_sling_shifts
                 WHERE target_month = ?"
            ).map_err(err)?;
            for row in stmt2.query_map(duckdb::params![target_month], |r| {
                let date: String = r.get(0)?;
                let start: String = r.get(1)?;
                let end: String = r.get(2)?;
                let uid: Option<i32> = r.get(3)?;
                let pid: i32 = r.get(4)?;
                Ok(serde_json::json!({
                    "type": "shift",
                    "dtstart": crate::sling::shift_iso(&date, &start),
                    "dtend": crate::sling::shift_iso(&date, &end),
                    "user": uid.map(|u| serde_json::json!({"id": u})),
                    "position": {"id": pid},
                    "location": {"id": studio_cfg.home_location_id},
                }))
            }).map_err(err)? { events.push(row.map_err(err)?); }
            events
        };

        // The algorithm builds its weekly slot template from the trailing
        // 3 months of shifts (propose.py:279). With no history, slot_rule
        // is empty and the result is a blank calendar. Fail loudly rather
        // than silently producing zero shifts.
        if history_events.is_empty() {
            return Err(format!(
                "No trailing-history shifts available for {target_month}. \
                 Click \"Pull from Sling\" on this month first so the algorithm \
                 has a slot template to work from."
            ));
        }

        serde_json::json!({
            "target_month": target_month,
            "home_location_id": studio_cfg.home_location_id,
            "teachers": teachers,
            "users": users_with_groups,
            "history_events": history_events,
            "month_events": month_events,
        })
    };
    Ok(payload_json)
}

/// Spawn a propose script (baseline or a versioned copy) with the payload
/// on stdin; parse its JSON output. Returns (parsed output, stderr tail).
fn spawn_propose(
    script_path: &std::path::Path,
    workdir: &std::path::Path,
    payload_json: &serde_json::Value,
    target_month: &str,
) -> Result<(ProposeOutput, String), String> {
    use std::io::Write;
    use std::process::Stdio;

    // Probed once and cached (py -3 / python / python3, Store stub and
    // version checked); Err is an actionable install message.
    let python = crate::python::resolve()?;
    let script = script_path
        .to_str()
        .ok_or_else(|| "script path is not valid UTF-8".to_string())?;
    let mut child = python.command()
        .args([script, "--json-out", "--from-stdin", "--target-month", target_month])
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn {}: {e}", python.display()))?;

    {
        let stdin = child.stdin.as_mut().ok_or_else(|| "no stdin".to_string())?;
        stdin.write_all(payload_json.to_string().as_bytes())
            .map_err(|e| format!("failed to write stdin: {e}"))?;
    }
    let output = child.wait_with_output()
        .map_err(|e| format!("failed to wait on the propose script: {e}"))?;

    let stderr_tail = tail(&String::from_utf8_lossy(&output.stderr), 40);
    if !output.status.success() {
        return Err(format!(
            "propose script exited {}:\n{}",
            output.status, stderr_tail
        ));
    }

    let parsed: ProposeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("invalid JSON from the propose script: {e}"))?;
    Ok((parsed, stderr_tail))
}

// `async` attribute: Tauri 2 runs plain sync commands on the main (UI)
// thread. Everything that spawns python, does HTTP, or otherwise takes
// seconds is marked `#[tauri::command(async)]` so it runs on the async
// runtime's thread pool instead and the window keeps painting.
#[tauri::command(async)]
pub fn generate_proposal(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    target_month: String,
    name: Option<String>,
) -> Result<GenerateResult, String> {
    // Fail fast on a bad draft name, before spending seconds in python.
    let name = name
        .map(|n| crate::drafts::clean_name(&n))
        .transpose()?;
    // Step 1: build the payload and resolve the ACTIVE algorithm version
    // (rules + script; app_settings pointer, see algorithm.rs).
    let project_root = find_project_root(&app).map_err(err)?;
    let (payload_json, script_path) = {
        let conn = db.0.lock().map_err(err)?;
        let mut payload = build_propose_payload(&conn, &target_month)?;
        let active = crate::algorithm::active_version(&conn)?;
        if let Some(v) = &active {
            payload["rules"] = v.rules.clone();
            payload["version_label"] = serde_json::Value::String(format!("v{}", v.version));
        }
        let dir = crate::algorithm::algorithms_dir(&app)?;
        let script =
            crate::algorithm::resolve_active_script(&dir, active.as_ref(), &project_root)?;
        (payload, script)
    };

    let (payload, stderr_tail) =
        spawn_propose(&script_path, &script_workdir(&app).map_err(err)?, &payload_json, &target_month)?;

    // Step 2: write the proposal + shifts to DuckDB in a single transaction.
    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;

    // Demote any prior "current" proposal for this month. is_current now only
    // means "newest generated"; which draft gets pushed is the month's push
    // draft (drafts.rs), which a new generate sets only if the month has none.
    tx.execute(
        "UPDATE proposals SET is_current = FALSE WHERE target_month = ?",
        duckdb::params![&payload.target_month],
    )
    .map_err(err)?;

    let parameters_json = serde_json::to_string(&payload.parameters).map_err(err)?;

    let proposal_id: i64 = tx
        .query_row(
            "INSERT INTO proposals (target_month, algorithm_version, parameters, is_current)
             VALUES (?, ?, ?, TRUE)
             RETURNING id",
            duckdb::params![
                &payload.target_month,
                &payload.algorithm_version,
                &parameters_json,
            ],
            |r| r.get(0),
        )
        .map_err(err)?;
    crate::drafts::record_generated(&tx, proposal_id, &payload.target_month, name.as_deref())?;

    let mut dropped_count = 0usize;
    for s in &payload.shifts {
        if s.is_dropped {
            dropped_count += 1;
        }
        tx.execute(
            "INSERT INTO proposal_shifts (
                proposal_id, shift_date, start_time, end_time,
                sling_position_id, sling_user_id, generation_reason,
                flag, is_coteach, coteach_label, is_dropped
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                proposal_id,
                &s.shift_date,
                &s.start_time,
                &s.end_time,
                s.sling_position_id,
                s.sling_user_id,
                &s.generation_reason,
                if s.flag.is_empty() { None } else { Some(s.flag.as_str()) },
                s.is_coteach,
                if s.coteach_label.is_empty() { None } else { Some(s.coteach_label.as_str()) },
                s.is_dropped,
            ],
        )
        .map_err(err)?;
    }

    tx.commit().map_err(err)?;
    // Force a checkpoint so the WAL doesn't accumulate state across runs.
    // If the binary dies mid-write later, replay only has to deal with the
    // single in-flight transaction (which DuckDB handles cleanly), not a
    // mountain of pending changes.
    let _ = conn.execute("CHECKPOINT", []);

    Ok(GenerateResult {
        proposal_id,
        target_month: payload.target_month,
        algorithm_version: payload.algorithm_version,
        shift_count: payload.shifts.len(),
        dropped_count,
        stderr_tail,
    })
}

#[derive(Serialize)]
pub struct ProposalSummary {
    pub id: i64,
    pub target_month: String,
    pub algorithm_version: String,
    pub generated_at: String,
    pub is_current: bool,
    pub shift_count: i64,
    pub dropped_count: i64,
    pub edit_count: i64,
    /// Draft metadata (proposal_drafts, migration 0012).
    pub name: String,
    pub archived: bool,
    pub parent_proposal_id: Option<i64>,
    pub created_from: String,
    /// The month's push draft (month_push_candidate) — the only draft Push sends.
    pub is_push_candidate: bool,
    /// At least one push to Sling is on record for this draft.
    pub pushed: bool,
    /// Live Sling shifts this draft owns (push_sync tracking, migration 0013).
    pub sling_shift_count: i64,
}

/// Shared SELECT for ProposalSummary rows (list + get). Callers append
/// WHERE / ORDER BY.
const PROPOSAL_SUMMARY_SQL: &str = "SELECT
        p.id,
        p.target_month,
        p.algorithm_version,
        CAST(p.generated_at AS VARCHAR),
        p.is_current,
        (SELECT count(*) FROM proposal_shifts ps WHERE ps.proposal_id = p.id) AS shift_count,
        (SELECT count(*) FROM proposal_shifts ps WHERE ps.proposal_id = p.id AND ps.is_dropped) AS dropped_count,
        (SELECT count(*) FROM edits e
            JOIN proposal_shifts ps2 ON ps2.id = e.proposal_shift_id
            WHERE ps2.proposal_id = p.id AND NOT e.reverted) AS edit_count,
        COALESCE(d.name, 'Draft #' || CAST(p.id AS VARCHAR)),
        COALESCE(d.archived, FALSE),
        d.parent_proposal_id,
        COALESCE(d.created_from, 'generate'),
        (m.proposal_id IS NOT NULL),
        EXISTS (SELECT 1 FROM pushes x WHERE x.proposal_id = p.id)
     FROM proposals p
     LEFT JOIN proposal_drafts d ON d.proposal_id = p.id
     LEFT JOIN month_push_candidate m
        ON m.target_month = p.target_month AND m.proposal_id = p.id";

fn summary_from_row(r: &duckdb::Row<'_>) -> duckdb::Result<ProposalSummary> {
    Ok(ProposalSummary {
        id: r.get(0)?,
        target_month: r.get(1)?,
        algorithm_version: r.get(2)?,
        generated_at: r.get(3)?,
        is_current: r.get(4)?,
        shift_count: r.get(5)?,
        dropped_count: r.get(6)?,
        edit_count: r.get(7)?,
        name: r.get(8)?,
        archived: r.get(9)?,
        parent_proposal_id: r.get(10)?,
        created_from: r.get(11)?,
        is_push_candidate: r.get(12)?,
        pushed: r.get(13)?,
        sling_shift_count: 0, // filled by with_sling_counts
    })
}

/// Fill sling_shift_count from push tracking (not expressible as a column
/// subquery: "latest tracking row per Sling shift" is a window query).
fn with_sling_counts(conn: &duckdb::Connection, mut list: Vec<ProposalSummary>) -> Result<Vec<ProposalSummary>, String> {
    let counts = crate::push_sync::live_counts(conn)?;
    for p in &mut list {
        p.sling_shift_count = counts.get(&p.id).copied().unwrap_or(0);
    }
    Ok(list)
}

#[derive(Serialize)]
pub struct ProposalShiftRow {
    pub id: i64,
    pub shift_date: String,
    pub start_time: String,
    pub end_time: String,
    pub class_name: String,
    pub sling_position_id: i32,
    pub teacher_name: Option<String>,
    pub sling_user_id: Option<i32>,
    pub generation_reason: String,
    pub flag: Option<String>,
    pub is_coteach: bool,
    pub coteach_label: Option<String>,
    pub is_dropped: bool,
}

#[derive(Serialize)]
pub struct ProposalDetail {
    pub summary: ProposalSummary,
    pub shifts: Vec<ProposalShiftRow>,
    pub is_stale: bool,
    pub last_pulled_at: Option<String>,
    /// Last check_draft_conflicts run against pulled data (migration 0013).
    pub last_checked_at: Option<String>,
}

/// A draft is stale when the month's latest pull (or availability refresh)
/// is newer than both its generation and its last conflict check — i.e.
/// nothing has looked at this draft against the current Sling data.
pub(crate) fn staleness(
    conn: &duckdb::Connection,
    proposal_id: i64,
) -> Result<(bool, Option<String>, Option<String>), String> {
    conn.query_row(
        "SELECT
            COALESCE(mp.pulled_at > greatest(p.generated_at, COALESCE(dc.checked_at, p.generated_at)), FALSE),
            CAST(mp.pulled_at AS VARCHAR),
            CAST(dc.checked_at AS VARCHAR)
         FROM proposals p
         LEFT JOIN month_pulls mp ON mp.target_month = p.target_month
         LEFT JOIN draft_checks dc ON dc.proposal_id = p.id
         WHERE p.id = ?",
        duckdb::params![proposal_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .map_err(err)
}

#[tauri::command]
pub fn list_proposals(db: State<'_, Db>) -> Result<Vec<ProposalSummary>, String> {
    let conn = db.0.lock().map_err(err)?;
    // Newest CREATED first (a duplicate keeps its parent's generated_at).
    let mut stmt = conn
        .prepare(&format!("{PROPOSAL_SUMMARY_SQL} ORDER BY p.id DESC"))
        .map_err(err)?;
    let rows = stmt.query_map([], summary_from_row).map_err(err)?;
    let list = rows.collect::<Result<Vec<_>, _>>().map_err(err)?;
    with_sling_counts(&conn, list)
}

#[tauri::command]
pub fn get_proposal(
    db: State<'_, Db>,
    proposal_id: i64,
) -> Result<ProposalDetail, String> {
    let conn = db.0.lock().map_err(err)?;

    let summary: ProposalSummary = conn
        .query_row(
            &format!("{PROPOSAL_SUMMARY_SQL} WHERE p.id = ?"),
            duckdb::params![proposal_id],
            summary_from_row,
        )
        .map_err(err)?;
    let summary = with_sling_counts(&conn, vec![summary])?.remove(0);

    let mut stmt = conn
        .prepare(
            "SELECT
                ps.id,
                CAST(ps.shift_date AS VARCHAR),
                ps.start_time,
                ps.end_time,
                pos.class_name,
                ps.sling_position_id,
                t.display_name,
                ps.sling_user_id,
                ps.generation_reason,
                ps.flag,
                ps.is_coteach,
                ps.coteach_label,
                ps.is_dropped
             FROM proposal_shifts ps
             JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
             LEFT JOIN teachers t ON t.sling_user_id = ps.sling_user_id
             WHERE ps.proposal_id = ?
             ORDER BY ps.shift_date, ps.start_time",
        )
        .map_err(err)?;

    let shifts = stmt
        .query_map(duckdb::params![proposal_id], |r| {
            Ok(ProposalShiftRow {
                id: r.get(0)?,
                shift_date: r.get(1)?,
                start_time: r.get(2)?,
                end_time: r.get(3)?,
                class_name: r.get(4)?,
                sling_position_id: r.get(5)?,
                teacher_name: r.get(6)?,
                sling_user_id: r.get(7)?,
                generation_reason: r.get(8)?,
                flag: r.get(9)?,
                is_coteach: r.get(10)?,
                coteach_label: r.get(11)?,
                is_dropped: r.get(12)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;

    let (is_stale, last_pulled_at, last_checked_at) = staleness(&conn, proposal_id)?;

    Ok(ProposalDetail { summary, shifts, is_stale, last_pulled_at, last_checked_at })
}

// ============================================================
// Manual edits to a proposal
// ============================================================

#[derive(Serialize)]
pub struct EditRow {
    pub id: i64,
    pub proposal_shift_id: i64,
    pub shift_date: String,
    pub start_time: String,
    pub class_name: String,
    pub field: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub old_teacher_name: Option<String>,
    pub new_teacher_name: Option<String>,
    pub old_class_name: Option<String>,
    pub new_class_name: Option<String>,
    pub reason: Option<String>,
    pub edited_at: String,
    pub reverted: bool,
}

/// Change the assigned teacher on a single proposal_shift. Records the
/// before/after in the `edits` table so we have full audit + rollback.
/// `new_user_id = None` means "drop this slot" (matches is_dropped).
/// Co-teach rows are blocked here — they need a separate flow that
/// expands the partner row.
#[tauri::command]
pub fn edit_proposal_shift_teacher(
    db: State<'_, Db>,
    proposal_shift_id: i64,
    new_user_id: Option<i32>,
    reason: Option<String>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;

    // Pull current state — error if the row doesn't exist or is co-teach.
    let (old_user_id, is_coteach): (Option<i32>, bool) = tx
        .query_row(
            "SELECT sling_user_id, is_coteach
             FROM proposal_shifts WHERE id = ?",
            duckdb::params![proposal_shift_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| format!("proposal_shift {proposal_shift_id} not found: {e:#}"))?;

    if is_coteach {
        return Err("co-teach editing is not yet supported".into());
    }
    if old_user_id == new_user_id {
        return Err("teacher unchanged".into());
    }

    let reason_clean = reason.and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t.to_string()) }
    });

    tx.execute(
        "INSERT INTO edits (proposal_shift_id, field, old_value, new_value, reason)
         VALUES (?, 'sling_user_id', ?, ?, ?)",
        duckdb::params![
            proposal_shift_id,
            old_user_id.map(|x| x.to_string()),
            new_user_id.map(|x| x.to_string()),
            reason_clean,
        ],
    )
    .map_err(err)?;

    tx.execute(
        "UPDATE proposal_shifts
         SET sling_user_id = ?, is_dropped = ?
         WHERE id = ?",
        duckdb::params![new_user_id, new_user_id.is_none(), proposal_shift_id],
    )
    .map_err(err)?;

    tx.commit().map_err(err)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(())
}

fn add_minutes_hhmm(hhmm: &str, minutes: i64) -> Result<String, String> {
    let (h, m) = hhmm
        .split_once(':')
        .ok_or_else(|| format!("bad time '{hhmm}'"))?;
    let h: i64 = h.parse().map_err(|_| format!("bad time '{hhmm}'"))?;
    let m: i64 = m.parse().map_err(|_| format!("bad time '{hhmm}'"))?;
    let total = (h * 60 + m + minutes).rem_euclid(24 * 60);
    Ok(format!("{:02}:{:02}", total / 60, total % 60))
}

/// Change the class format on a single proposal_shift. Records the
/// before/after position ids in `edits` (field 'sling_position_id') and
/// recomputes end_time from the new class's duration. Co-teach rows are
/// blocked, like teacher edits.
fn edit_position_impl(
    conn: &mut duckdb::Connection,
    proposal_shift_id: i64,
    new_position_id: i32,
    reason: Option<String>,
) -> Result<(), String> {
    let tx = conn.transaction().map_err(err)?;

    let (old_pid, start_time, is_coteach): (i32, String, bool) = tx
        .query_row(
            "SELECT sling_position_id, start_time, is_coteach
             FROM proposal_shifts WHERE id = ?",
            duckdb::params![proposal_shift_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| format!("proposal_shift {proposal_shift_id} not found: {e:#}"))?;

    if is_coteach {
        return Err("co-teach editing is not yet supported".into());
    }
    if old_pid == new_position_id {
        return Err("class type unchanged".into());
    }

    let (duration, active): (i32, bool) = tx
        .query_row(
            "SELECT duration_minutes, active FROM positions WHERE sling_position_id = ?",
            duckdb::params![new_position_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| format!("position {new_position_id} not found: {e:#}"))?;
    if !active {
        return Err("that class type is not schedulable".into());
    }
    let end_time = add_minutes_hhmm(&start_time, duration as i64)?;

    let reason_clean = reason.and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t.to_string()) }
    });

    tx.execute(
        "INSERT INTO edits (proposal_shift_id, field, old_value, new_value, reason)
         VALUES (?, 'sling_position_id', ?, ?, ?)",
        duckdb::params![
            proposal_shift_id,
            old_pid.to_string(),
            new_position_id.to_string(),
            reason_clean
        ],
    )
    .map_err(err)?;

    tx.execute(
        "UPDATE proposal_shifts SET sling_position_id = ?, end_time = ? WHERE id = ?",
        duckdb::params![new_position_id, end_time, proposal_shift_id],
    )
    .map_err(err)?;

    tx.commit().map_err(err)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(())
}

#[tauri::command]
pub fn edit_proposal_shift_position(
    db: State<'_, Db>,
    proposal_shift_id: i64,
    new_position_id: i32,
    reason: Option<String>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(err)?;
    edit_position_impl(&mut conn, proposal_shift_id, new_position_id, reason)
}

#[tauri::command]
pub fn list_edits_for_proposal(
    db: State<'_, Db>,
    proposal_id: i64,
) -> Result<Vec<EditRow>, String> {
    let conn = db.0.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT
                e.id,
                e.proposal_shift_id,
                CAST(ps.shift_date AS VARCHAR),
                ps.start_time,
                pos.class_name,
                e.field,
                e.old_value,
                e.new_value,
                t_old.display_name AS old_teacher_name,
                t_new.display_name AS new_teacher_name,
                p_old.class_name AS old_class_name,
                p_new.class_name AS new_class_name,
                e.reason,
                CAST(e.edited_at AS VARCHAR),
                e.reverted
             FROM edits e
             JOIN proposal_shifts ps ON ps.id = e.proposal_shift_id
             JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
             LEFT JOIN teachers t_old
                ON e.field = 'sling_user_id'
               AND CAST(t_old.sling_user_id AS VARCHAR) = e.old_value
             LEFT JOIN teachers t_new
                ON e.field = 'sling_user_id'
               AND CAST(t_new.sling_user_id AS VARCHAR) = e.new_value
             LEFT JOIN positions p_old
                ON e.field = 'sling_position_id'
               AND CAST(p_old.sling_position_id AS VARCHAR) = e.old_value
             LEFT JOIN positions p_new
                ON e.field = 'sling_position_id'
               AND CAST(p_new.sling_position_id AS VARCHAR) = e.new_value
             WHERE ps.proposal_id = ?
             ORDER BY e.edited_at DESC",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(duckdb::params![proposal_id], |r| {
            Ok(EditRow {
                id: r.get(0)?,
                proposal_shift_id: r.get(1)?,
                shift_date: r.get(2)?,
                start_time: r.get(3)?,
                class_name: r.get(4)?,
                field: r.get(5)?,
                old_value: r.get(6)?,
                new_value: r.get(7)?,
                old_teacher_name: r.get(8)?,
                new_teacher_name: r.get(9)?,
                old_class_name: r.get(10)?,
                new_class_name: r.get(11)?,
                reason: r.get(12)?,
                edited_at: r.get(13)?,
                reverted: r.get(14)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

// ============================================================
// Anthropic key management + app settings
// ============================================================

/// Model allowlist for the Claude features. Exact ids only — an unknown or
/// retired stored value (e.g. an older claude-opus-4-8 setting) falls back
/// to the default at call time. Keep in sync with SettingsScreen.tsx.
pub const CLAUDE_MODELS: &[&str] = &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-4-5"];
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-5-5";

pub fn claude_model(conn: &duckdb::Connection) -> String {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'claude_model'",
            [],
            |r| r.get(0),
        )
        .ok();
    match stored {
        Some(m) if CLAUDE_MODELS.contains(&m.as_str()) => m,
        _ => DEFAULT_CLAUDE_MODEL.to_string(),
    }
}

#[tauri::command]
pub fn get_app_setting(db: State<'_, Db>, key: String) -> Result<Option<String>, String> {
    let conn = db.0.lock().map_err(err)?;
    Ok(conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?",
            duckdb::params![key],
            |r| r.get(0),
        )
        .ok())
}

#[tauri::command]
pub fn set_app_setting(db: State<'_, Db>, key: String, value: String) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?, ?, now())",
        duckdb::params![key, value],
    )
    .map_err(err)?;
    Ok(())
}

#[tauri::command(async)]
pub fn set_anthropic_key(
    key: State<'_, AnthropicKey>,
    secrets: State<'_, crate::secrets::Secrets>,
    value: String,
) -> Result<(), String> {
    let trimmed = value.trim();
    {
        let mut guard = key.0.lock().map_err(err)?;
        *guard = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    // Persist to Stronghold so the key survives app restarts (same
    // mechanism as the Sling token).
    if trimmed.is_empty() {
        secrets
            .remove(crate::secrets::KEY_ANTHROPIC)
            .map_err(|e| e.to_string())?;
    } else {
        secrets
            .set(crate::secrets::KEY_ANTHROPIC, trimmed)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn has_anthropic_key(key: State<'_, AnthropicKey>) -> Result<bool, String> {
    let guard = key.0.lock().map_err(err)?;
    Ok(guard.is_some())
}

// ============================================================
// Claude review of a proposal + its edits
// ============================================================

#[derive(Serialize)]
pub struct ReviewResult {
    pub run_id: i64,
    pub suggestions: Vec<review::ReviewSuggestion>,
    pub overall_assessment: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_input_tokens: u32,
    pub cost_usd: f64,
    pub duration_ms: u32,
}

#[derive(Serialize)]
pub struct ReviewRunSummary {
    pub id: i64,
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cost_usd: f64,
    pub duration_ms: i32,
    pub ran_at: String,
    pub suggestions: Vec<review::ReviewSuggestion>,
    pub overall_assessment: String,
}

#[tauri::command(async)]
pub fn review_proposal(
    db: State<'_, Db>,
    key: State<'_, AnthropicKey>,
    proposal_id: i64,
) -> Result<ReviewResult, String> {
    // 1. Lock+copy the API key, then drop the lock immediately so we don't
    //    hold it across the long-running HTTP call.
    let api_key = {
        let guard = key.0.lock().map_err(err)?;
        guard
            .clone()
            .ok_or_else(|| "Anthropic API key is not set — paste it on the Settings tab".to_string())?
    };

    // 2. Build the user payload from DB. Same pattern: lock, query, drop.
    let (user_payload, model) = {
        let conn = db.0.lock().map_err(err)?;
        let model = claude_model(&conn);

        let (target_month, algorithm_version): (String, String) = conn
            .query_row(
                "SELECT target_month, algorithm_version FROM proposals WHERE id = ?",
                duckdb::params![proposal_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| format!("proposal {proposal_id} not found: {e:#}"))?;

        let mut shifts_stmt = conn
            .prepare(
                "SELECT
                    CAST(ps.shift_date AS VARCHAR),
                    ps.start_time,
                    ps.end_time,
                    pos.class_name,
                    t.display_name,
                    ps.coteach_label,
                    ps.generation_reason,
                    ps.flag,
                    ps.is_dropped
                 FROM proposal_shifts ps
                 JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
                 LEFT JOIN teachers t ON t.sling_user_id = ps.sling_user_id
                 WHERE ps.proposal_id = ?
                 ORDER BY ps.shift_date, ps.start_time",
            )
            .map_err(err)?;
        let shifts: Vec<serde_json::Value> = shifts_stmt
            .query_map(duckdb::params![proposal_id], |r| {
                let teacher: Option<String> = r.get(4)?;
                let coteach_label: Option<String> = r.get(5)?;
                let flag: Option<String> = r.get(7)?;
                let is_dropped: bool = r.get(8)?;
                Ok(json!({
                    "date": r.get::<_, String>(0)?,
                    "start": r.get::<_, String>(1)?,
                    "end": r.get::<_, String>(2)?,
                    "class": r.get::<_, String>(3)?,
                    "teacher": coteach_label.or(teacher),
                    "reason": r.get::<_, String>(6)?,
                    "flag": flag.unwrap_or_default(),
                    "dropped": is_dropped,
                }))
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;

        let mut edits_stmt = conn
            .prepare(
                "SELECT
                    CAST(ps.shift_date AS VARCHAR),
                    ps.start_time,
                    pos.class_name,
                    t_old.display_name,
                    t_new.display_name,
                    e.reason
                 FROM edits e
                 JOIN proposal_shifts ps ON ps.id = e.proposal_shift_id
                 JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
                 LEFT JOIN teachers t_old ON CAST(t_old.sling_user_id AS VARCHAR) = e.old_value
                 LEFT JOIN teachers t_new ON CAST(t_new.sling_user_id AS VARCHAR) = e.new_value
                 WHERE ps.proposal_id = ? AND NOT e.reverted
                 ORDER BY e.edited_at",
            )
            .map_err(err)?;
        let edits: Vec<serde_json::Value> = edits_stmt
            .query_map(duckdb::params![proposal_id], |r| {
                let from: Option<String> = r.get(3)?;
                let to: Option<String> = r.get(4)?;
                let reason: Option<String> = r.get(5)?;
                Ok(json!({
                    "date": r.get::<_, String>(0)?,
                    "start": r.get::<_, String>(1)?,
                    "class": r.get::<_, String>(2)?,
                    "from": from.unwrap_or_else(|| "DROPPED".into()),
                    "to": to.unwrap_or_else(|| "DROPPED".into()),
                    "reason": reason,
                }))
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;

        let mut roster_stmt = conn
            .prepare(
                "SELECT display_name, weekly_target, weekly_max, is_lead, variety_multiplier
                 FROM teachers WHERE active
                 ORDER BY is_lead DESC, display_name",
            )
            .map_err(err)?;
        let roster: Vec<serde_json::Value> = roster_stmt
            .query_map([], |r| {
                Ok(json!({
                    "name": r.get::<_, String>(0)?,
                    "weekly_target": r.get::<_, i32>(1)?,
                    "weekly_max": r.get::<_, i32>(2)?,
                    "is_lead": r.get::<_, bool>(3)?,
                    "variety_multiplier": r.get::<_, f64>(4)?,
                }))
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;

        (json!({
            "proposal": {
                "id": proposal_id,
                "target_month": target_month,
                "algorithm_version": algorithm_version,
            },
            "shifts": shifts,
            "edits": edits,
            "roster": roster,
        }), model)
    };

    // 3. Call Anthropic. This is the slow step (~5–30s).
    let result = review::run_review(&api_key, &model, &user_payload).map_err(err)?;

    // 4. Persist run for audit + cost tracking.
    let suggestions_json = serde_json::to_string(&result.payload).map_err(err)?;
    let conn = db.0.lock().map_err(err)?;
    let run_id: i64 = conn
        .query_row(
            "INSERT INTO claude_runs (
                proposal_id, model, input_tokens, output_tokens,
                input_text, output_text, cost_usd, duration_ms
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
            duckdb::params![
                proposal_id,
                &result.model,
                result.input_tokens as i32,
                result.output_tokens as i32,
                &result.raw_input,
                &suggestions_json,
                result.cost_usd,
                result.duration_ms as i32,
            ],
            |r| r.get(0),
        )
        .map_err(err)?;
    let _ = conn.execute("CHECKPOINT", []);

    Ok(ReviewResult {
        run_id,
        suggestions: result.payload.suggestions,
        overall_assessment: result.payload.overall_assessment,
        model: result.model,
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
        cache_read_input_tokens: result.cache_read_input_tokens,
        cost_usd: result.cost_usd,
        duration_ms: result.duration_ms,
    })
}

#[tauri::command]
pub fn list_reviews_for_proposal(
    db: State<'_, Db>,
    proposal_id: i64,
) -> Result<Vec<ReviewRunSummary>, String> {
    let conn = db.0.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, model, input_tokens, output_tokens, cost_usd, duration_ms,
                    CAST(ran_at AS VARCHAR), output_text
             FROM claude_runs
             WHERE proposal_id = ?
             ORDER BY ran_at DESC",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(duckdb::params![proposal_id], |r| {
            let output_text: String = r.get(7)?;
            // Re-parse the stored payload. claude_runs also holds editor
            // runs (different JSON shape) — mark those to be filtered out
            // below instead of failing the whole query.
            let parsed: review::ReviewPayload = serde_json::from_str(&output_text)
                .unwrap_or(review::ReviewPayload {
                    suggestions: vec![],
                    overall_assessment: "__not_a_review__".into(),
                });
            // duckdb-rs returns DECIMAL as a string; parse to f64 so the
            // frontend can display it cleanly.
            let cost_str: String = r.get(4)?;
            let cost_usd = cost_str.parse::<f64>().unwrap_or(0.0);
            Ok(ReviewRunSummary {
                id: r.get(0)?,
                model: r.get(1)?,
                input_tokens: r.get(2)?,
                output_tokens: r.get(3)?,
                cost_usd,
                duration_ms: r.get(5)?,
                ran_at: r.get(6)?,
                suggestions: parsed.suggestions,
                overall_assessment: parsed.overall_assessment,
            })
        })
        .map_err(err)?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?
        .into_iter()
        .filter(|r| r.overall_assessment != "__not_a_review__")
        .collect())
}

// ============================================================
// Claude proposal editor (spec: 2026-07-06-claude-proposal-editor-design)
// ============================================================

#[derive(Serialize)]
pub struct ClaudeEditResult {
    pub run_id: i64,
    pub summary: String,
    pub edits: Vec<crate::editor::ProposedEdit>,
    pub ruleset_proposal: Option<crate::editor::RulesetProposal>,
    pub needs_code_change: Option<crate::editor::NeedsCodeChange>,
    pub model: String,
    pub cost_usd: f64,
    pub duration_ms: u32,
}

/// Everything the editor prompt needs about a proposal, in one JSON value.
/// Shared by the editor and code-draft calls.
fn build_editor_payload(
    conn: &duckdb::Connection,
    proposal_id: i64,
    instruction: &str,
) -> Result<serde_json::Value, String> {
    let target_month: String = conn
        .query_row(
            "SELECT target_month FROM proposals WHERE id = ?",
            duckdb::params![proposal_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("proposal {proposal_id} not found: {e:#}"))?;

    let shifts: Vec<serde_json::Value> = {
        let mut stmt = conn
            .prepare(
                "SELECT ps.id, CAST(ps.shift_date AS VARCHAR), ps.start_time, ps.end_time,
                        pos.class_name, t.display_name, ps.sling_user_id, ps.is_coteach,
                        ps.is_dropped
                 FROM proposal_shifts ps
                 JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
                 LEFT JOIN teachers t ON t.sling_user_id = ps.sling_user_id
                 WHERE ps.proposal_id = ?
                 ORDER BY ps.shift_date, ps.start_time",
            )
            .map_err(err)?;
        stmt.query_map(duckdb::params![proposal_id], |r| {
            Ok(json!({
                "proposal_shift_id": r.get::<_, i64>(0)?,
                "date": r.get::<_, String>(1)?,
                "start": r.get::<_, String>(2)?,
                "end": r.get::<_, String>(3)?,
                "class_name": r.get::<_, String>(4)?,
                "teacher": r.get::<_, Option<String>>(5)?,
                "sling_user_id": r.get::<_, Option<i32>>(6)?,
                "is_coteach": r.get::<_, bool>(7)?,
                "is_dropped": r.get::<_, bool>(8)?,
            }))
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?
    };

    let roster: Vec<serde_json::Value> = {
        let mut stmt = conn
            .prepare(
                "SELECT sling_user_id, display_name, weekly_target, weekly_max
                 FROM teachers WHERE active ORDER BY display_name",
            )
            .map_err(err)?;
        stmt.query_map([], |r| {
            Ok(json!({
                "sling_user_id": r.get::<_, i32>(0)?,
                "name": r.get::<_, String>(1)?,
                "weekly_target": r.get::<_, i32>(2)?,
                "weekly_max": r.get::<_, i32>(3)?,
            }))
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?
    };

    let qualifications: Vec<serde_json::Value> = {
        let mut stmt = conn
            .prepare(
                "SELECT tq.sling_user_id, p.class_name
                 FROM teacher_qualifications tq
                 JOIN positions p ON p.sling_position_id = tq.sling_position_id
                 WHERE NOT tq.is_blocklisted",
            )
            .map_err(err)?;
        stmt.query_map([], |r| {
            Ok(json!({
                "sling_user_id": r.get::<_, i32>(0)?,
                "class_name": r.get::<_, String>(1)?,
            }))
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?
    };

    let blocks: Vec<serde_json::Value> = query_availability_blocks(conn, &target_month)?
        .into_iter()
        .map(|b| {
            json!({
                "sling_user_id": b.sling_user_id,
                "source": b.source,
                "starts_at": b.starts_at,
                "ends_at": b.ends_at,
            })
        })
        .collect();

    let edit_history: Vec<serde_json::Value> = {
        let mut stmt = conn
            .prepare(
                "SELECT CAST(ps.shift_date AS VARCHAR), ps.start_time, pos.class_name,
                        e.field, e.old_value, e.new_value, e.reason
                 FROM edits e
                 JOIN proposal_shifts ps ON ps.id = e.proposal_shift_id
                 JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
                 WHERE ps.proposal_id = ? AND NOT e.reverted
                 ORDER BY e.edited_at",
            )
            .map_err(err)?;
        stmt.query_map(duckdb::params![proposal_id], |r| {
            Ok(json!({
                "date": r.get::<_, String>(0)?,
                "start": r.get::<_, String>(1)?,
                "class_name": r.get::<_, String>(2)?,
                "field": r.get::<_, String>(3)?,
                "from": r.get::<_, Option<String>>(4)?,
                "to": r.get::<_, Option<String>>(5)?,
                "reason": r.get::<_, Option<String>>(6)?,
            }))
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?
    };

    let active_rules = crate::algorithm::active_version(conn)?
        .map(|v| v.rules)
        .unwrap_or_else(|| json!({}));

    // Every class name a rule may reference (rules are validated against
    // this list — see algorithm::validate_rules_in_context).
    let class_names: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT class_name FROM positions ORDER BY class_name")
            .map_err(err)?;
        stmt.query_map([], |r| r.get(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
    };

    Ok(json!({
        "proposal": { "id": proposal_id, "target_month": target_month, "shifts": shifts },
        "roster": roster,
        "class_names": class_names,
        "qualifications": qualifications,
        "availability_blocks": blocks,
        "edit_history": edit_history,
        "active_rules": active_rules,
        "instruction": instruction,
    }))
}

/// Check Claude's proposed edits against the database. Invalid edits are
/// kept (so the user sees what was attempted) but marked un-appliable.
fn validate_claude_edits(
    conn: &duckdb::Connection,
    proposal_id: i64,
    edits: &mut [crate::editor::ProposedEdit],
) -> Result<(), String> {
    use std::collections::{HashMap, HashSet};

    let mut shift_info: HashMap<i64, (bool, Option<i32>, i32)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, is_coteach, sling_user_id, sling_position_id
                 FROM proposal_shifts WHERE proposal_id = ?",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(duckdb::params![proposal_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    (r.get::<_, bool>(1)?, r.get::<_, Option<i32>>(2)?, r.get::<_, i32>(3)?),
                ))
            })
            .map_err(err)?;
        for row in rows {
            let (k, v) = row.map_err(err)?;
            shift_info.insert(k, v);
        }
    }

    let active_teachers: HashSet<i32> = {
        let mut stmt = conn
            .prepare("SELECT sling_user_id FROM teachers WHERE active")
            .map_err(err)?;
        stmt.query_map([], |r| r.get(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
    };

    let class_to_pid: HashMap<String, i32> = {
        let mut stmt = conn
            .prepare("SELECT class_name, sling_position_id FROM positions WHERE active")
            .map_err(err)?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i32>(1)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
    };

    for e in edits.iter_mut() {
        let fail = |note: String| (false, Some(note));
        let (valid, note) = match shift_info.get(&e.proposal_shift_id) {
            None => fail("that slot is not in this proposal".to_string()),
            Some((is_coteach, current_uid, current_pid)) => {
                if *is_coteach {
                    fail("co-teach slots can't be edited here".to_string())
                } else {
                    match e.action.as_str() {
                        "reassign" => match e.new_user_id {
                            None => fail("reassign needs new_user_id".to_string()),
                            Some(uid) if !active_teachers.contains(&uid) => {
                                fail(format!("teacher {uid} is unknown or inactive"))
                            }
                            Some(uid) if Some(uid) == *current_uid => {
                                fail("already assigned to that teacher".to_string())
                            }
                            Some(_) => (true, None),
                        },
                        "unassign" => {
                            if current_uid.is_none() {
                                fail("already unassigned".to_string())
                            } else {
                                (true, None)
                            }
                        }
                        "change_format" => match &e.new_class_name {
                            None => fail("change_format needs new_class_name".to_string()),
                            Some(name) => match class_to_pid.get(name) {
                                None => fail(format!("'{name}' is not a schedulable class")),
                                Some(pid) if pid == current_pid => {
                                    fail("already that format".to_string())
                                }
                                Some(_) => (true, None),
                            },
                        },
                        other => fail(format!("unknown action '{other}'")),
                    }
                }
            }
        };
        e.valid = valid;
        e.validation_note = note;
    }
    Ok(())
}

fn record_run_target(conn: &duckdb::Connection, run_id: i64, proposal_id: i64) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO claude_run_targets (claude_run_id, proposal_id) VALUES (?, ?)",
        duckdb::params![run_id, proposal_id],
    )
    .map_err(err)?;
    Ok(())
}

fn persist_claude_run(
    conn: &duckdb::Connection,
    proposal_id: i64,
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
    raw_input: &str,
    raw_output: &str,
    cost_usd: f64,
    duration_ms: u32,
) -> Result<i64, String> {
    conn.query_row(
        "INSERT INTO claude_runs (
            proposal_id, model, input_tokens, output_tokens,
            input_text, output_text, cost_usd, duration_ms
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        duckdb::params![
            proposal_id,
            model,
            input_tokens as i32,
            output_tokens as i32,
            raw_input,
            raw_output,
            cost_usd,
            duration_ms as i32,
        ],
        |r| r.get(0),
    )
    .map_err(err)
}

#[tauri::command(async)]
pub fn claude_edit_proposal(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    key: State<'_, AnthropicKey>,
    proposal_id: i64,
    instruction: String,
    group_run_id: Option<i64>,
) -> Result<ClaudeEditResult, String> {
    if instruction.trim().is_empty() {
        return Err("instruction is empty".to_string());
    }
    let api_key = {
        let guard = key.0.lock().map_err(err)?;
        guard
            .clone()
            .ok_or_else(|| "Anthropic API key is not set — add it in Settings".to_string())?
    };

    // Lock, build payload, drop — never hold the DB across the HTTP call.
    let (user_payload, model) = {
        let conn = db.0.lock().map_err(err)?;
        (
            build_editor_payload(&conn, proposal_id, instruction.trim())?,
            claude_model(&conn),
        )
    };

    let system = crate::editor::editor_system_prompt(find_project_root(&app).ok().as_deref());
    let result = crate::editor::run_editor(&api_key, &model, &system, &user_payload).map_err(err)?;

    let conn = db.0.lock().map_err(err)?;
    let mut payload = result.payload;
    validate_claude_edits(&conn, proposal_id, &mut payload.edits)?;

    // A rule proposal that doesn't validate (schema, HH:MM, unknown teacher
    // ids or class names) is downgraded to a summary note rather than shown
    // with a broken Adopt button.
    let mut summary = payload.summary.clone();
    let rule_ctx = crate::algorithm::load_rule_context(&conn)?;
    let ruleset_proposal = match payload.ruleset_proposal {
        Some(rp) => match crate::algorithm::validate_rules_in_context(&rp.rules, &rule_ctx) {
            Ok(_) => Some(rp),
            Err(e) => {
                summary.push_str(&format!(
                    " (A rule change was proposed but failed validation and was dropped: {e})"
                ));
                None
            }
        },
        None => None,
    };

    let run_id = persist_claude_run(
        &conn,
        proposal_id,
        &result.model,
        result.input_tokens,
        result.output_tokens,
        &result.raw_input,
        &result.raw_output,
        result.cost_usd,
        result.duration_ms,
    )?;
    // One prompt sent to several drafts = one run per draft, all linked under
    // the prompt's first run (group_run_id); a single-draft run links to itself.
    record_run_target(&conn, group_run_id.unwrap_or(run_id), proposal_id)?;
    let _ = conn.execute("CHECKPOINT", []);

    Ok(ClaudeEditResult {
        run_id,
        summary,
        edits: payload.edits,
        ruleset_proposal,
        needs_code_change: payload.needs_code_change,
        model: result.model,
        cost_usd: result.cost_usd,
        duration_ms: result.duration_ms,
    })
}

// ============================================================
// Candidate algorithms: code drafts (tier 3), plus the shared
// "reproduce last month" validation + diffs that every candidate — rules
// or code — goes through before Adopt (schedule-algorithm skill).
// ============================================================

/// Read a script with line endings normalised to \n, so Claude's
/// search/replace edits and the diff view don't trip over CRLF checkouts.
fn read_script(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map(|s| s.replace("\r\n", "\n"))
        .map_err(|e| format!("could not read {}: {e}", path.display()))
}

#[derive(Serialize)]
pub struct CodeDraft {
    pub run_id: i64,
    pub description: String,
    /// The full resulting script (active script with Claude's edits applied).
    pub script: String,
    /// Unified diff: active script → draft.
    pub diff: String,
    pub edit_count: usize,
    /// The active rules at draft time — carried into the code version on
    /// adopt so adopting code never drops the standing rule set.
    pub rules: serde_json::Value,
    pub model: String,
    pub cost_usd: f64,
    pub duration_ms: u32,
}

#[tauri::command(async)]
pub fn claude_draft_code_change(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    key: State<'_, AnthropicKey>,
    proposal_id: i64,
    instruction: String,
    rationale: String,
) -> Result<CodeDraft, String> {
    let api_key = {
        let guard = key.0.lock().map_err(err)?;
        guard
            .clone()
            .ok_or_else(|| "Anthropic API key is not set — add it in Settings".to_string())?
    };
    let project_root = find_project_root(&app).map_err(err)?;
    let dir = crate::algorithm::algorithms_dir(&app)?;

    let (user_payload, model, current_script, active_rules) = {
        let conn = db.0.lock().map_err(err)?;
        let model = claude_model(&conn);
        let target_month: String = conn
            .query_row(
                "SELECT target_month FROM proposals WHERE id = ?",
                duckdb::params![proposal_id],
                |r| r.get(0),
            )
            .map_err(|e| format!("proposal {proposal_id} not found: {e:#}"))?;
        let active = crate::algorithm::active_version(&conn)?;
        let script_path =
            crate::algorithm::resolve_active_script(&dir, active.as_ref(), &project_root)?;
        let current_script = read_script(&script_path)?;
        let active_rules = active.map(|v| v.rules).unwrap_or_else(|| json!({}));
        (
            json!({
                "target_month": target_month,
                "active_rules": active_rules,
                "instruction": instruction,
                "rationale": rationale,
            }),
            model,
            current_script,
            active_rules,
        )
    };

    let result = crate::editor::run_code_draft(&api_key, &model, &current_script, &user_payload)
        .map_err(err)?;

    // Log the run (cost audit) before applying: a failed apply still cost money.
    let run_id = {
        let conn = db.0.lock().map_err(err)?;
        let id = persist_claude_run(
            &conn,
            proposal_id,
            &result.model,
            result.input_tokens,
            result.output_tokens,
            &result.raw_input,
            &result.raw_output,
            result.cost_usd,
            result.duration_ms,
        )?;
        record_run_target(&conn, id, proposal_id)?;
        let _ = conn.execute("CHECKPOINT", []);
        id
    };

    let script = crate::editor::apply_edit_blocks(&current_script, &result.payload.edits)
        .map_err(|e| {
            format!(
                "Claude's edits could not be applied to the active script: {e}\n\
                 (Logged as Claude run #{run_id}; drafting again usually fixes this.)"
            )
        })?;
    let diff = crate::textdiff::unified_diff(
        &current_script,
        &script,
        "active/propose.py",
        "draft/propose.py",
        3,
    );

    Ok(CodeDraft {
        run_id,
        description: result.payload.description,
        script,
        diff,
        edit_count: result.payload.edits.len(),
        rules: active_rules,
        model: result.model,
        cost_usd: result.cost_usd,
        duration_ms: result.duration_ms,
    })
}

/// One shift of a propose.py run, reduced to what the per-slot diff needs.
#[derive(Debug, Clone)]
pub(crate) struct RunShift {
    pub(crate) date: String,
    pub(crate) start: String,
    pub(crate) position_id: i32,
    pub(crate) class_name: String,
    pub(crate) user_id: Option<i32>,
    pub(crate) coteach_label: String,
    pub(crate) dropped: bool,
}

impl RunShift {
    fn from_output(s: &ProposeShift) -> Self {
        RunShift {
            date: s.shift_date.clone(),
            start: s.start_time.clone(),
            position_id: s.sling_position_id,
            class_name: s.class_name.clone(),
            user_id: s.sling_user_id,
            coteach_label: s.coteach_label.clone(),
            dropped: s.is_dropped,
        }
    }

    /// Who teaches it. A co-teach row is identified by its label (both
    /// teachers), so a change of either co-teacher counts as a change.
    fn assignee(&self) -> (bool, &str, Option<i32>) {
        (self.dropped, self.coteach_label.as_str(), self.user_id)
    }

    fn teacher_label(&self, names: &std::collections::HashMap<i32, String>) -> String {
        if self.dropped {
            "Dropped".to_string()
        } else if !self.coteach_label.is_empty() {
            self.coteach_label.clone()
        } else {
            match self.user_id {
                Some(u) => names.get(&u).cloned().unwrap_or_else(|| format!("teacher {u}")),
                None => "Unassigned".to_string(),
            }
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SlotChange {
    pub date: String,
    pub weekday: String,
    pub start: String,
    /// "changed" (same slot, different class and/or teacher) | "added" | "removed"
    pub kind: String,
    pub class_before: Option<String>,
    pub class_after: Option<String>,
    pub teacher_before: Option<String>,
    pub teacher_after: Option<String>,
    /// Added/removed slots explained by a time-shift rule change.
    pub expected: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct CandidateValidation {
    /// "pass" | "needs_confirm" (too many changed assignments and/or slots
    /// that appeared/disappeared with no time-shift rule change to explain
    /// them — adopt only after explicit confirmation) | "error" (a script
    /// failed or the rules don't validate — never adoptable)
    pub status: String,
    pub error: Option<String>,
    pub reasons: Vec<String>,
    pub month: String,
    pub slot_count: i64,
    pub candidate_slot_count: i64,
    pub changed_count: i64,
    pub added_count: i64,
    pub removed_count: i64,
    pub unexpected_count: i64,
    pub changed_pct: f64,
    pub changes: Vec<SlotChange>,
}

/// Share of slots whose assignment may change before adoption needs an
/// explicit "adopt anyway".
const CHANGE_THRESHOLD: f64 = 0.25;

impl CandidateValidation {
    fn error(month: &str, message: String) -> Self {
        CandidateValidation {
            status: "error".to_string(),
            error: Some(message),
            reasons: Vec::new(),
            month: month.to_string(),
            slot_count: 0,
            candidate_slot_count: 0,
            changed_count: 0,
            added_count: 0,
            removed_count: 0,
            unexpected_count: 0,
            changed_pct: 0.0,
            changes: Vec::new(),
        }
    }
}

pub(crate) fn weekday_of(date: &str) -> String {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.format("%a").to_string())
        .unwrap_or_default()
}

/// Start times (per weekend day) whose slots may legitimately appear or
/// disappear because the candidate changes a sat/sun time-shift entry.
fn expected_moves(
    active: &serde_json::Value,
    candidate: &serde_json::Value,
) -> std::collections::HashMap<&'static str, std::collections::HashSet<String>> {
    use std::collections::{BTreeMap, HashMap, HashSet};
    let mut out: HashMap<&'static str, HashSet<String>> = HashMap::new();
    for (key, day) in [("sat_time_shifts", "Sat"), ("sun_time_shifts", "Sun")] {
        let read = |v: &serde_json::Value| -> BTreeMap<String, String> {
            v.get(key)
                .and_then(|m| m.as_object())
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        };
        let (a, c) = (read(active), read(candidate));
        let set = out.entry(day).or_default();
        for k in a.keys().chain(c.keys()) {
            if a.get(k) != c.get(k) {
                set.insert(k.clone());
                set.extend(a.get(k).cloned());
                set.extend(c.get(k).cloned());
            }
        }
    }
    out
}

/// Per-slot diff of two runs on the same payload. Slots are keyed by
/// (date, start); within a slot, rows pair up by identical assignment
/// first, then by position (teacher change), then in order (class change).
/// Co-teach rows keep their own identity (the label), never collapsed.
pub(crate) fn compare_runs(
    month: &str,
    baseline: &[RunShift],
    candidate: &[RunShift],
    names: &std::collections::HashMap<i32, String>,
    moves: &std::collections::HashMap<&'static str, std::collections::HashSet<String>>,
) -> CandidateValidation {
    use std::collections::BTreeMap;
    type Side<'a> = (Vec<&'a RunShift>, Vec<&'a RunShift>);
    let mut slots: BTreeMap<(String, String), Side<'_>> = BTreeMap::new();
    for b in baseline {
        slots.entry((b.date.clone(), b.start.clone())).or_default().0.push(b);
    }
    for c in candidate {
        slots.entry((c.date.clone(), c.start.clone())).or_default().1.push(c);
    }

    let mut changes = Vec::new();
    let (mut changed, mut added, mut removed, mut unexpected) = (0i64, 0i64, 0i64, 0i64);
    for ((date, start), (mut bs, mut cs)) in slots {
        let weekday = weekday_of(&date);
        // 1. Identical rows are unchanged.
        bs.retain(|b| {
            match cs
                .iter()
                .position(|c| c.position_id == b.position_id && c.assignee() == b.assignee())
            {
                Some(i) => {
                    cs.remove(i);
                    false
                }
                None => true,
            }
        });
        // 2. Same class, different teacher; 3. anything left pairs in order.
        let mut pairs: Vec<(&RunShift, &RunShift)> = Vec::new();
        bs.retain(|b| match cs.iter().position(|c| c.position_id == b.position_id) {
            Some(i) => {
                pairs.push((*b, cs.remove(i)));
                false
            }
            None => true,
        });
        while !bs.is_empty() && !cs.is_empty() {
            pairs.push((bs.remove(0), cs.remove(0)));
        }
        let expected = moves.get(weekday.as_str()).is_some_and(|s| s.contains(&start));
        for (b, c) in pairs {
            changed += 1;
            changes.push(SlotChange {
                date: date.clone(),
                weekday: weekday.clone(),
                start: start.clone(),
                kind: "changed".to_string(),
                class_before: Some(b.class_name.clone()),
                class_after: Some(c.class_name.clone()),
                teacher_before: Some(b.teacher_label(names)),
                teacher_after: Some(c.teacher_label(names)),
                expected: false,
            });
        }
        for b in bs {
            removed += 1;
            if !expected {
                unexpected += 1;
            }
            changes.push(SlotChange {
                date: date.clone(),
                weekday: weekday.clone(),
                start: start.clone(),
                kind: "removed".to_string(),
                class_before: Some(b.class_name.clone()),
                class_after: None,
                teacher_before: Some(b.teacher_label(names)),
                teacher_after: None,
                expected,
            });
        }
        for c in cs {
            added += 1;
            if !expected {
                unexpected += 1;
            }
            changes.push(SlotChange {
                date: date.clone(),
                weekday: weekday.clone(),
                start: start.clone(),
                kind: "added".to_string(),
                class_before: None,
                class_after: Some(c.class_name.clone()),
                teacher_before: None,
                teacher_after: Some(c.teacher_label(names)),
                expected,
            });
        }
    }

    let slot_count = baseline.len() as i64;
    let changed_pct = changed as f64 / slot_count.max(1) as f64;
    let mut reasons = Vec::new();
    // Both unexplained slot changes and a large share of changed assignments
    // are adoptable only after an explicit confirm (a code change may add or
    // remove a class on purpose); script errors never are.
    if unexpected > 0 {
        reasons.push(format!(
            "{unexpected} slot(s) appeared or disappeared that no time-shift rule change explains"
        ));
    }
    if changed_pct > CHANGE_THRESHOLD {
        reasons.push(format!(
            "{changed} of {slot_count} assignments change ({:.0}% — more than {:.0}%)",
            changed_pct * 100.0,
            CHANGE_THRESHOLD * 100.0
        ));
    }
    let status = if reasons.is_empty() { "pass" } else { "needs_confirm" };
    CandidateValidation {
        status: status.to_string(),
        error: None,
        reasons,
        month: month.to_string(),
        slot_count,
        candidate_slot_count: candidate.len() as i64,
        changed_count: changed,
        added_count: added,
        removed_count: removed,
        unexpected_count: unexpected,
        changed_pct,
        changes,
    }
}

/// Run the active (script + rules) and the candidate (script + rules) on the
/// same stdin payload, in parallel, and diff them per slot. The baseline is
/// a fresh run of the active algorithm — not the stored (possibly hand-
/// edited) proposal — so only the candidate's own effect shows up.
#[allow(clippy::too_many_arguments)]
fn validate_candidate(
    month: &str,
    payload: &serde_json::Value,
    workdir: &std::path::Path,
    active_script: &std::path::Path,
    active_rules: &serde_json::Value,
    candidate_script: &std::path::Path,
    candidate_rules: &serde_json::Value,
    names: &std::collections::HashMap<i32, String>,
) -> CandidateValidation {
    let with_rules = |rules: &serde_json::Value, label: &str| {
        let mut p = payload.clone();
        p["rules"] = rules.clone();
        p["version_label"] = json!(label);
        p
    };
    let (pa, pc) = (with_rules(active_rules, "active"), with_rules(candidate_rules, "candidate"));
    let (ra, rc) = std::thread::scope(|s| {
        let ha = s.spawn(|| spawn_propose(active_script, workdir, &pa, month));
        let hc = s.spawn(|| spawn_propose(candidate_script, workdir, &pc, month));
        let join = |h: std::thread::ScopedJoinHandle<'_, Result<(ProposeOutput, String), String>>| {
            h.join().unwrap_or_else(|_| Err("the propose run panicked".to_string()))
        };
        (join(ha), join(hc))
    });
    let candidate_out = match rc {
        Ok((out, _)) => out,
        Err(e) => {
            return CandidateValidation::error(
                month,
                format!("The candidate failed when re-running {month}: {e}"),
            )
        }
    };
    let active_out = match ra {
        Ok((out, _)) => out,
        Err(e) => {
            return CandidateValidation::error(
                month,
                format!("The active algorithm failed on {month}, so there is no baseline to compare against: {e}"),
            )
        }
    };
    let base: Vec<RunShift> = active_out.shifts.iter().map(RunShift::from_output).collect();
    let cand: Vec<RunShift> = candidate_out.shifts.iter().map(RunShift::from_output).collect();
    compare_runs(month, &base, &cand, names, &expected_moves(active_rules, candidate_rules))
}

#[derive(Serialize)]
pub struct CandidatePreview {
    pub active_version: i32,
    pub rules_diff: Vec<crate::algorithm::RuleDiffEntry>,
    /// Unified diff vs the active script; None for rules-only candidates.
    pub script_diff: Option<String>,
    pub validation: CandidateValidation,
}

/// Everything the Adopt card shows before Adopt: rules diff and script diff
/// vs the ACTIVE version, and a "reproduce last month" run of both on the
/// most recently generated month. `script_content` None = rules-only
/// candidate (runs on the active script, as adoption would).
#[tauri::command(async)]
pub fn preview_algorithm_candidate(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    rules: serde_json::Value,
    script_content: Option<String>,
) -> Result<CandidatePreview, String> {
    let project_root = find_project_root(&app).map_err(err)?;
    let dir = crate::algorithm::algorithms_dir(&app)?;

    let (active, rules_check, month, payload, names) = {
        let conn = db.0.lock().map_err(err)?;
        let active = crate::algorithm::active_version(&conn)?;
        let ctx = crate::algorithm::load_rule_context(&conn)?;
        let rules_check = crate::algorithm::validate_rules_in_context(&rules, &ctx).map(|_| ());
        let month: Option<String> = conn
            .query_row(
                "SELECT target_month FROM proposals ORDER BY generated_at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .ok();
        let payload = month.as_ref().map(|m| build_propose_payload(&conn, m));
        let names: std::collections::HashMap<i32, String> = {
            let mut stmt = conn
                .prepare("SELECT sling_user_id, display_name FROM teachers")
                .map_err(err)?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(err)?
                .collect::<Result<_, _>>()
                .map_err(err)?
        };
        (active, rules_check, month, payload, names)
    };

    let active_rules = active.as_ref().map(|v| v.rules.clone()).unwrap_or_else(|| json!({}));
    let active_script_path =
        crate::algorithm::resolve_active_script(&dir, active.as_ref(), &project_root)?;
    let active_script = read_script(&active_script_path)?;
    let candidate_script = script_content.map(|s| s.replace("\r\n", "\n"));

    let rules_diff = crate::algorithm::diff_rules(&active_rules, &rules);
    let script_diff = candidate_script.as_ref().map(|s| {
        crate::textdiff::unified_diff(&active_script, s, "active/propose.py", "candidate/propose.py", 3)
    });

    let month_label = month.clone().unwrap_or_default();
    let validation = match (rules_check, payload) {
        (Err(e), _) => CandidateValidation::error(&month_label, format!("The rules don't validate: {e}")),
        (Ok(()), None) => CandidateValidation::error(
            &month_label,
            "No proposals yet — generate one first so there is a month to reproduce.".to_string(),
        ),
        (Ok(()), Some(Err(e))) => CandidateValidation::error(&month_label, e),
        (Ok(()), Some(Ok(payload))) => {
            let workdir = script_workdir(&app).map_err(err)?;
            // Unique temp name: two previews may run at once.
            let temp = candidate_script.as_ref().map(|_| {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default();
                dir.join(format!("candidate_{}_{nanos}.py", std::process::id()))
            });
            if let (Some(path), Some(content)) = (&temp, &candidate_script) {
                std::fs::write(path, content).map_err(err)?;
            }
            let v = validate_candidate(
                &month_label,
                &payload,
                &workdir,
                &active_script_path,
                &active_rules,
                temp.as_deref().unwrap_or(&active_script_path),
                &rules,
                &names,
            );
            if let Some(path) = &temp {
                let _ = std::fs::remove_file(path);
            }
            v
        }
    };

    Ok(CandidatePreview {
        active_version: active.map(|v| v.version).unwrap_or(crate::algorithm::BASELINE_VERSION),
        rules_diff,
        script_diff,
        validation,
    })
}

// ============================================================
// Sling token (in-memory cache; Stronghold is the persistence layer)
// ============================================================

#[tauri::command(async)]
pub fn set_sling_token(
    token: State<'_, SlingToken>,
    secrets: State<'_, crate::secrets::Secrets>,
    value: String,
) -> Result<(), String> {
    let trimmed = value.trim();
    {
        let mut t = token.0.lock().map_err(err)?;
        *t = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    // Persist to Stronghold so the token survives app restarts.
    if trimmed.is_empty() {
        secrets
            .remove(crate::secrets::KEY_SLING_TOKEN)
            .map_err(|e| e.to_string())?;
    } else {
        secrets
            .set(crate::secrets::KEY_SLING_TOKEN, trimmed)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn has_sling_token(token: State<'_, SlingToken>) -> Result<bool, String> {
    let t = token.0.lock().map_err(err)?;
    Ok(t.is_some())
}

// ============================================================
// Sling credentials (email + password) — saved in Stronghold,
// injected into the login webview to pre-fill the form. Captcha
// and the submit click stay with the user.
//
// The credentials are deliberately write-only from JS's perspective:
// there's no get_sling_credentials command. They flow only into the
// login webview's init script via sling_login.rs.
// ============================================================

#[tauri::command(async)]
pub fn set_sling_credentials(
    secrets: State<'_, crate::secrets::Secrets>,
    email: String,
    password: String,
) -> Result<(), String> {
    let email = email.trim();
    if email.is_empty() {
        secrets
            .remove(crate::secrets::KEY_SLING_EMAIL)
            .map_err(|e| e.to_string())?;
        secrets
            .remove(crate::secrets::KEY_SLING_PASSWORD)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    secrets
        .set(crate::secrets::KEY_SLING_EMAIL, email)
        .map_err(|e| e.to_string())?;
    secrets
        .set(crate::secrets::KEY_SLING_PASSWORD, &password)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn has_sling_credentials(
    secrets: State<'_, crate::secrets::Secrets>,
) -> Result<bool, String> {
    let has_email = secrets
        .get(crate::secrets::KEY_SLING_EMAIL)
        .map_err(|e| e.to_string())?
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    Ok(has_email)
}

// ============================================================
// Sling pull — fetch + write to DuckDB transactionally
// ============================================================

#[derive(serde::Serialize, Clone)]
pub struct RosterSyncSummary {
    pub teachers_active: i64,
    pub teachers_deactivated: i64,
    pub positions_active: i64,
    pub positions_deactivated: i64,
    pub qualifications: i64,
}

/// Reconcile the roster + positions + qualifications against Sling (source of
/// truth). Active home-location users qualified for a schedulable position are
/// imported; departed teachers and removed positions are deactivated (never
/// deleted — schedule history references them). App-only fields (teacher
/// caps/variety/ranking/notes; position duration/is_special/active) are
/// preserved. Must run inside a transaction.
fn sync_roster(
    conn: &duckdb::Connection,
    users: &[crate::sling::SlingUser],
    groups: &[crate::sling::SlingGroup],
    cfg: &crate::sling::StudioConfig,
) -> Result<RosterSyncSummary, String> {
    use std::collections::HashSet;

    // 1. Positions from Sling position-type groups. Compare-before-write:
    // read the current rows once, then only touch rows that actually change.
    // No-op UPDATEs waste WAL and needlessly exercise DuckDB's touchy
    // UPDATE machinery (see migrations 0003/0004/0009).
    let pos_groups: Vec<(i64, String)> = groups.iter()
        .filter(|g| g.kind == "position")
        .map(|g| (g.id, g.name.clone()))
        .collect();
    let sling_pos_ids: HashSet<i64> = pos_groups.iter().map(|(id, _)| *id).collect();
    let existing_pos: std::collections::HashMap<i32, (String, bool)> = {
        let mut s = conn.prepare(
            "SELECT sling_position_id, class_name, active FROM positions").map_err(err)?;
        s.query_map([], |r| Ok((r.get::<_, i32>(0)?, (r.get::<_, String>(1)?, r.get::<_, bool>(2)?))))
            .map_err(err)?.collect::<Result<_, _>>().map_err(err)?
    };
    for (id, name) in &pos_groups {
        let pid = *id as i32;
        match existing_pos.get(&pid) {
            Some((current_name, _)) => {
                // `active` is user-managed (schedulable toggle) — never
                // re-activate here; only track renames.
                if current_name != name {
                    conn.execute("UPDATE positions SET class_name = ? WHERE sling_position_id = ?",
                        duckdb::params![name, pid]).map_err(err)?;
                }
            }
            None => {
                conn.execute(
                    "INSERT INTO positions (sling_position_id, class_name, duration_minutes, is_special, active)
                     VALUES (?, ?, 60, FALSE, TRUE)",
                    duckdb::params![pid, name]).map_err(err)?;
            }
        }
    }
    let mut positions_deactivated = 0i64;
    for (pid, (_, active)) in &existing_pos {
        if *active && !sling_pos_ids.contains(&(*pid as i64)) {
            conn.execute("UPDATE positions SET active = FALSE WHERE sling_position_id = ?",
                duckdb::params![pid]).map_err(err)?;
            positions_deactivated += 1;
        }
    }

    // 2. Schedulable position set (active positions).
    let schedulable: HashSet<i64> = {
        let mut s = conn.prepare("SELECT sling_position_id FROM positions WHERE active = TRUE").map_err(err)?;
        s.query_map([], |r| r.get::<_, i32>(0)).map_err(err)?
            .collect::<Result<Vec<_>, _>>().map_err(err)?
            .into_iter().map(|p| p as i64).collect()
    };
    let positions_active = schedulable.len() as i64;

    // 3. Teachers. Same compare-before-write shape as positions.
    struct TeacherRow {
        display_name: String,
        locations: Option<String>,
        active: bool,
        is_lead: bool,
    }
    let existing_teachers: std::collections::HashMap<i32, TeacherRow> = {
        let mut s = conn.prepare(
            "SELECT sling_user_id, display_name, locations, active, is_lead FROM teachers").map_err(err)?;
        s.query_map([], |r| Ok((
            r.get::<_, i32>(0)?,
            TeacherRow {
                display_name: r.get(1)?,
                locations: r.get(2)?,
                active: r.get(3)?,
                is_lead: r.get(4)?,
            },
        ))).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
    };
    let location_names = crate::sling::location_name_by_id(groups);
    let mut imported: HashSet<i32> = HashSet::new();
    let mut teachers_active = 0i64;
    for u in users {
        if !crate::sling::is_schedulable_teacher(u, cfg.home_location_id, &schedulable) { continue; }
        let uid = u.id as i32;
        imported.insert(uid);
        teachers_active += 1;
        let display = format!("{} {}", u.name, u.lastname).trim().to_string();
        let locations = crate::sling::compute_locations(&u.group_ids, &location_names);
        let is_lead = u.id == cfg.acting_user_id;
        match existing_teachers.get(&uid) {
            Some(t) => {
                let unchanged = t.display_name == display
                    && t.locations == locations
                    && t.active
                    && t.is_lead == is_lead;
                if !unchanged {
                    conn.execute(
                        "UPDATE teachers SET display_name = ?, locations = ?, active = TRUE, is_lead = ?
                         WHERE sling_user_id = ?",
                        duckdb::params![display, locations, is_lead, uid]).map_err(err)?;
                }
            }
            None => {
                conn.execute(
                    "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max,
                        is_lead, ranking_weight, variety_multiplier, active, locations)
                     VALUES (?, ?, 4, 5, ?, 1.0, 1.0, TRUE, ?)",
                    duckdb::params![uid, display, is_lead, locations]).map_err(err)?;
            }
        }
    }
    let mut teachers_deactivated = 0i64;
    for (tid, t) in &existing_teachers {
        if t.active && !imported.contains(tid) {
            conn.execute("UPDATE teachers SET active = FALSE WHERE sling_user_id = ?",
                duckdb::params![tid]).map_err(err)?;
            teachers_deactivated += 1;
        }
    }

    // 4. Qualifications (imported teachers × schedulable positions).
    let mut sling_pairs: HashSet<(i32, i32)> = HashSet::new();
    for u in users {
        let uid = u.id as i32;
        if !imported.contains(&uid) { continue; }
        for g in &u.group_ids {
            if schedulable.contains(g) { sling_pairs.insert((uid, *g as i32)); }
        }
    }
    let existing: Vec<(i32, i32, bool)> = {
        let mut s = conn.prepare(
            "SELECT sling_user_id, sling_position_id, is_blocklisted FROM teacher_qualifications").map_err(err)?;
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(err)?
            .collect::<Result<_, _>>().map_err(err)?
    };
    for (uid, pid, blocked) in &existing {
        if *blocked { continue; }
        if !sling_pairs.contains(&(*uid, *pid)) {
            conn.execute("DELETE FROM teacher_qualifications WHERE sling_user_id = ? AND sling_position_id = ?",
                duckdb::params![uid, pid]).map_err(err)?;
        }
    }
    let mut qualifications = 0i64;
    for (uid, pid) in &sling_pairs {
        conn.execute(
            "INSERT INTO teacher_qualifications (sling_user_id, sling_position_id)
             VALUES (?, ?) ON CONFLICT DO NOTHING",
            duckdb::params![uid, pid]).map_err(err)?;
        qualifications += 1;
    }

    Ok(RosterSyncSummary { teachers_active, teachers_deactivated, positions_active, positions_deactivated, qualifications })
}

/// Replace one month's availability/leave blocks and external shifts with
/// the calendar events Sling just returned. Shared by the full pull and the
/// availability refresh. Must run inside a transaction.
fn write_month_events(
    tx: &duckdb::Connection,
    target_month: &str,
    month_events: &[crate::sling::CalendarEvent],
    roster_ids: &std::collections::HashSet<i32>,
    cfg: &crate::sling::StudioConfig,
) -> Result<(i64, i64), String> {
    let (m_start, m_end) = sling::month_range(target_month).map_err(err)?;
    tx.execute(
        "DELETE FROM availability_blocks
         WHERE starts_at >= CAST(? AS TIMESTAMPTZ) AND starts_at <= CAST(? AS TIMESTAMPTZ)",
        duckdb::params![&m_start, &m_end],
    ).map_err(err)?;
    let mut availability_count: i64 = 0;
    for e in month_events {
        if e.kind != "availability" && e.kind != "leave" { continue; }
        // Ownership guard: this pull owns (deletes + rewrites) only blocks
        // that START in the target month — the same window the DELETE above
        // clears. A spanning block Sling returns for a later month would
        // otherwise be inserted a second time.
        if e.dtstart.get(0..7) != Some(target_month) { continue; }
        let uid = match e.user.as_ref().or_else(|| e.users.as_ref().and_then(|v| v.first())) {
            Some(u) => u.id as i32,
            None => continue,
        };
        if !roster_ids.contains(&uid) { continue; }
        tx.execute(
            "INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at)
             VALUES (?, ?, CAST(? AS TIMESTAMPTZ), CAST(? AS TIMESTAMPTZ))",
            duckdb::params![uid, &e.kind, &e.dtstart, &e.dtend],
        ).map_err(err)?;
        availability_count += 1;
    }

    tx.execute(
        "DELETE FROM external_sling_shifts WHERE target_month = ?",
        duckdb::params![target_month],
    ).map_err(err)?;
    let mut external_shift_count: i64 = 0;
    let home_location_shifts = sling::filter_events(month_events, &["shift"], cfg.home_location_id);
    for e in home_location_shifts {
        let shift_id = match e.id { Some(v) => v, None => continue };
        let date_part = e.dtstart.get(0..10).unwrap_or("").to_string();
        let start_hm = e.dtstart.get(11..16).unwrap_or("").to_string();
        let end_hm = e.dtend.get(11..16).unwrap_or("").to_string();
        let pid = match e.position.as_ref() { Some(p) => p.id as i32, None => continue };
        let uid = e.user.as_ref().or_else(|| e.users.as_ref().and_then(|v| v.first())).map(|u| u.id as i32);
        let status = e.status.clone().unwrap_or_else(|| "planning".to_string());
        tx.execute(
            "INSERT OR REPLACE INTO external_sling_shifts
                (sling_shift_id, target_month, shift_date, start_time, end_time,
                 sling_user_id, sling_position_id, status, pulled_at)
             VALUES (?, ?, CAST(? AS DATE), ?, ?, ?, ?, ?, now())",
            duckdb::params![shift_id, target_month, &date_part, &start_hm, &end_hm, uid, pid, &status],
        ).map_err(err)?;
        external_shift_count += 1;
    }
    Ok((availability_count, external_shift_count))
}

#[tauri::command(async)]
pub fn pull_month_from_sling(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    target_month: String,
) -> Result<PullResult, String> {
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — paste one in Settings".to_string())?
    };
    // Studio identifiers come from runtime config (migration 0007), not
    // compiled-in constants. Load before the network pull.
    let cfg = {
        let conn = db.0.lock().map_err(err)?;
        load_studio_config(&conn)?
    };
    if cfg.org_id == 0 || cfg.home_location_id == 0 {
        return Err(
            "Studio not configured — use “Set up studio” (it detects your org and \
             location from your Sling login) or enter the IDs in Settings → Studio configuration before pulling."
                .to_string(),
        );
    }
    let payload = sling::pull_month(&token_str, &target_month, &cfg).map_err(err)?;

    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;

    // Roster + positions + qualifications are reconciled from Sling here.
    let _roster = sync_roster(&tx, &payload.users, &payload.groups, &cfg)?;

    let roster_ids: std::collections::HashSet<i32> = {
        let mut s = tx.prepare("SELECT sling_user_id FROM teachers WHERE active = TRUE").map_err(err)?;
        s.query_map([], |r| r.get(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
    };

    let user_count: i64 = roster_ids.len() as i64;
    let qual_count: i64 = _roster.qualifications;

    let (availability_count, external_shift_count) =
        write_month_events(&tx, &target_month, &payload.month_events, &roster_ids, &cfg)?;
    let mut history_shift_count: i64 = 0;
    for e in &payload.history_shifts {
        let shift_id = match e.id { Some(v) => v, None => continue };
        let date_part = e.dtstart.get(0..10).unwrap_or("").to_string();
        let start_hm = e.dtstart.get(11..16).unwrap_or("").to_string();
        let end_hm = e.dtend.get(11..16).unwrap_or("").to_string();
        let pid = match e.position.as_ref() { Some(p) => p.id as i32, None => continue };
        let uid = e.user.as_ref().or_else(|| e.users.as_ref().and_then(|v| v.first())).map(|u| u.id as i32);
        let hist_month = date_part.get(0..7).unwrap_or("").to_string();
        if hist_month.is_empty() { continue; }
        tx.execute(
            "INSERT OR REPLACE INTO external_sling_shifts
                (sling_shift_id, target_month, shift_date, start_time, end_time,
                 sling_user_id, sling_position_id, status, pulled_at)
             VALUES (?, ?, CAST(? AS DATE), ?, ?, ?, ?, ?, now())",
            duckdb::params![shift_id, &hist_month, &date_part, &start_hm, &end_hm,
                            uid, pid, &"published".to_string()],
        ).map_err(err)?;
        history_shift_count += 1;
    }

    tx.execute(
        "INSERT OR REPLACE INTO month_pulls
            (target_month, pulled_at, user_count, qual_count, availability_count, external_shift_count)
         VALUES (?, now(), ?, ?, ?, ?)",
        duckdb::params![&target_month, user_count, qual_count, availability_count, external_shift_count],
    ).map_err(err)?;

    tx.commit().map_err(err)?;
    // Same WAL-bounding policy as generate/edit: checkpoint after the
    // largest write in the app so a later crash replays little.
    let _ = conn.execute("CHECKPOINT", []);

    Ok(PullResult {
        target_month: target_month.clone(),
        pulled_at: chrono::Utc::now().to_rfc3339(),
        user_count,
        qual_count,
        availability_count,
        external_shift_count,
        history_shift_count,
    })
}

// ============================================================
// Push proposal to Sling — spec building (the sync itself: push_sync.rs)
// ============================================================

/// Load proposal rows + roster map + studio config, then build the gated
/// push specs and the target month string. Shared by push_sync preview and execute.
pub(crate) fn build_specs_for_proposal(
    conn: &duckdb::Connection,
    proposal_id: i64,
) -> Result<(Vec<crate::sling::PushSpec>, crate::sling::StudioConfig, String), String> {
    // Only the month's push draft may be pushed (see drafts.rs).
    crate::drafts::ensure_push_candidate(conn, proposal_id)?;
    let studio_cfg = load_studio_config(conn)?;
    if studio_cfg.org_id == 0 || studio_cfg.home_location_id == 0 {
        return Err(
            "Studio not configured — use “Set up studio” (it detects your org and \
             location from your Sling login) or enter the IDs in Settings → Studio configuration before pushing."
                .to_string(),
        );
    }
    let target_month: String = conn
        .query_row(
            "SELECT target_month FROM proposals WHERE id = ?",
            duckdb::params![proposal_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("proposal {proposal_id} not found: {e}"))?;

    let name_to_id: std::collections::HashMap<String, i64> = {
        let mut stmt = conn
            .prepare("SELECT display_name, sling_user_id FROM teachers")
            .map_err(err)?;
        stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i32>(1)? as i64))
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?
    };

    let inputs: Vec<crate::sling::ProposalShiftInput> = {
        let mut stmt = conn
            .prepare(
                "SELECT ps.id, CAST(ps.shift_date AS VARCHAR), ps.start_time, ps.end_time,
                        ps.sling_position_id, ps.sling_user_id, t.display_name, pos.class_name,
                        ps.is_coteach, ps.coteach_label, ps.is_dropped
                 FROM proposal_shifts ps
                 JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
                 LEFT JOIN teachers t ON t.sling_user_id = ps.sling_user_id
                 WHERE ps.proposal_id = ?
                 ORDER BY ps.shift_date, ps.start_time",
            )
            .map_err(err)?;
        stmt.query_map(duckdb::params![proposal_id], |r| {
            let uid: Option<i32> = r.get(5)?;
            Ok(crate::sling::ProposalShiftInput {
                proposal_shift_id: r.get::<_, i64>(0)?,
                date: r.get(1)?,
                start: r.get(2)?,
                end: r.get(3)?,
                position_id: r.get::<_, i32>(4)? as i64,
                user_id: uid.map(|u| u as i64),
                class_name: r.get(7)?,
                is_coteach: r.get(8)?,
                coteach_label: r.get(9)?,
                is_dropped: r.get(10)?,
            })
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?
    };

    let specs = crate::sling::build_push_specs(&inputs, &name_to_id)?;
    Ok((specs, studio_cfg, target_month))
}

#[tauri::command]
pub fn import_external_shift(
    db: State<'_, Db>,
    sling_shift_id: i64,
    proposal_id: i64,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;
    let ext: (String, String, String, Option<i32>, i32) = tx.query_row(
        "SELECT CAST(shift_date AS VARCHAR), start_time, end_time, sling_user_id, sling_position_id
         FROM external_sling_shifts WHERE sling_shift_id = ?",
        duckdb::params![sling_shift_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    ).map_err(err)?;
    tx.execute(
        "INSERT INTO proposal_shifts
           (proposal_id, shift_date, start_time, end_time, sling_position_id,
            sling_user_id, generation_reason, flag, is_coteach, coteach_label, is_dropped)
         VALUES (?, CAST(? AS DATE), ?, ?, ?, ?, ?, '', FALSE, NULL, FALSE)",
        duckdb::params![proposal_id, &ext.0, &ext.1, &ext.2, ext.4, ext.3,
                        &"imported from external sling shift".to_string()],
    ).map_err(err)?;
    tx.commit().map_err(err)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(())
}

#[derive(Serialize)]
pub struct AvailabilityBlockRow {
    pub sling_user_id: i32,
    pub source: String,
    pub starts_at: String,
    pub ends_at: String,
}

/// Availability/leave blocks that OVERLAP the target month — not just the
/// ones that start inside it. A leave that begins in the previous month and
/// runs into this one must stay visible to the proposer and the issue queue,
/// or the teacher looks available for its first days.
fn query_availability_blocks(
    conn: &duckdb::Connection,
    target_month: &str,
) -> Result<Vec<AvailabilityBlockRow>, String> {
    let (start, end) = crate::sling::month_range(target_month).map_err(err)?;
    let mut stmt = conn.prepare(
        "SELECT sling_user_id, source, CAST(starts_at AS VARCHAR), CAST(ends_at AS VARCHAR)
         FROM availability_blocks
         WHERE starts_at <= CAST(? AS TIMESTAMPTZ) AND ends_at >= CAST(? AS TIMESTAMPTZ)"
    ).map_err(err)?;
    let rows = stmt.query_map(duckdb::params![&end, &start], |r| {
        Ok(AvailabilityBlockRow {
            sling_user_id: r.get(0)?,
            source: r.get(1)?,
            starts_at: r.get(2)?,
            ends_at: r.get(3)?,
        })
    }).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

#[tauri::command]
pub fn list_availability_blocks(
    db: State<'_, Db>,
    target_month: String,
) -> Result<Vec<AvailabilityBlockRow>, String> {
    let conn = db.0.lock().map_err(err)?;
    query_availability_blocks(&conn, &target_month)
}

#[derive(Serialize)]
pub struct ExternalShiftRow {
    pub sling_shift_id: i64,
    pub shift_date: String,
    pub start_time: String,
    pub end_time: String,
    pub sling_user_id: Option<i32>,
    pub sling_position_id: i32,
    pub status: String,
}

#[tauri::command]
pub fn list_external_shifts_for_month(
    db: State<'_, Db>,
    target_month: String,
) -> Result<Vec<ExternalShiftRow>, String> {
    let conn = db.0.lock().map_err(err)?;
    let mut stmt = conn.prepare(
        "SELECT sling_shift_id, CAST(shift_date AS VARCHAR), start_time, end_time,
                sling_user_id, sling_position_id, status
         FROM external_sling_shifts WHERE target_month = ?"
    ).map_err(err)?;
    let rows = stmt.query_map(duckdb::params![&target_month], |r| Ok(ExternalShiftRow {
        sling_shift_id: r.get(0)?,
        shift_date: r.get(1)?,
        start_time: r.get(2)?,
        end_time: r.get(3)?,
        sling_user_id: r.get(4)?,
        sling_position_id: r.get(5)?,
        status: r.get(6)?,
    })).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

// ============================================================
// Sling browser login flow
// ============================================================

#[tauri::command]
pub async fn open_sling_login_window(app: tauri::AppHandle) -> Result<(), String> {
    // Webview creation must NOT run on the main/UI thread. On Windows,
    // WebviewWindowBuilder::build() blocks while WebView2 asynchronously creates
    // its controller, and that controller-ready notification is only delivered
    // from the event loop's top-level message processing. Calling build() *on*
    // the main thread — directly from a sync command, OR via run_on_main_thread —
    // nests it inside a user-event callback, so the notification never arrives and
    // build() deadlocks: the window frame paints but its content never initializes
    // (and DevTools never opens). WebKitGTK on Linux has no async-controller step,
    // so this only bit on Windows.
    //
    // Making this command `async` runs it on the async runtime (a worker thread).
    // From there build() dispatches the actual creation to the event loop's
    // top-level context — where the controller wait can complete — and blocks the
    // worker, not the UI, until the window is ready. This is the pattern in
    // Tauri's own docs for opening a window from a command.
    crate::sling_login::open_login_window(app).map_err(err)
}

#[tauri::command(async)]
pub fn discover_studio_config(
    token: State<'_, SlingToken>,
    org_hint: State<'_, SlingOrgHint>,
) -> Result<crate::sling::DiscoveredStudio, String> {
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — log in to Sling first".to_string())?
    };
    let hint = { *org_hint.0.lock().map_err(err)? };
    crate::sling::discover_studio(&token_str, hint).map_err(err)
}

/// Detect the studio from the logged-in Sling user and apply the setup rule
/// (studio_setup::decide): autosave when the config is unset and detection is
/// unambiguous; otherwise report "ask" / "ok" / "mismatch" for the frontend.
/// Never overwrites a complete config. Runs off the UI thread.
#[tauri::command(async)]
pub fn auto_detect_studio_config(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    org_hint: State<'_, SlingOrgHint>,
) -> Result<crate::studio_setup::DetectOutcome, String> {
    use crate::studio_setup::{decide, Candidates, Decision, DetectOutcome};
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — log in to Sling first".to_string())?
    };
    let hint = { *org_hint.0.lock().map_err(err)? };
    // Network first, without holding the DB lock.
    let discovered = crate::sling::discover_studio(&token_str, hint).map_err(err)?;
    let conn = db.0.lock().map_err(err)?;
    let cfg = load_studio_config(&conn)?;
    let decision = decide(&cfg, &Candidates::from_discovered(&discovered));
    if let Decision::AutoSave { org_id, acting_user_id, home_location_id } = decision {
        write_studio_config(&conn, org_id, acting_user_id, home_location_id)?;
    }
    let after = load_studio_config(&conn)?;
    let reasons = match &decision {
        Decision::Mismatch { reasons } => reasons.clone(),
        _ => Vec::new(),
    };
    Ok(DetectOutcome {
        decision: decision.kind(),
        discovered,
        current: StudioConfigDto {
            org_id: after.org_id,
            acting_user_id: after.acting_user_id,
            home_location_id: after.home_location_id,
        },
        reasons,
    })
}

// ============================================================
// Standalone roster refresh — sync roster without pulling a month
// ============================================================

#[tauri::command(async)]
pub fn refresh_roster_from_sling(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
) -> Result<RosterSyncSummary, String> {
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — log in to Sling first".to_string())?
    };
    let cfg = {
        let conn = db.0.lock().map_err(err)?;
        load_studio_config(&conn)?
    };
    if cfg.org_id == 0 || cfg.home_location_id == 0 {
        return Err("Studio not configured — use “Set up studio” (it detects your org and \
                    location from your Sling login) or enter the IDs in Settings → Studio configuration before refreshing the roster.".to_string());
    }
    let users = crate::sling::fetch_users(&token_str).map_err(err)?;
    let groups = crate::sling::fetch_groups(&token_str).map_err(err)?;
    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;
    let summary = sync_roster(&tx, &users, &groups, &cfg)?;
    tx.commit().map_err(err)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(summary)
}

// ============================================================
// Availability refresh — re-pull availability/leave (+ roster, external
// shifts) for the current and future months WITHOUT regenerating drafts.
// Drafts keep their edits; check_draft_conflicts (conflicts.rs) then shows
// what the new availability breaks.
// ============================================================

#[derive(serde::Serialize, Clone)]
pub struct MonthRefresh {
    pub target_month: String,
    pub availability_count: i64,
    pub external_shift_count: i64,
}

#[derive(serde::Serialize, Clone)]
pub struct AvailabilityRefreshResult {
    pub months: Vec<MonthRefresh>,
    pub roster: RosterSyncSummary,
    pub refreshed_at: String,
}

/// Months to refresh: every month with a pull or a draft, from `current`
/// ("YYYY-MM") on. Past months are read-only and never touched.
fn refresh_months(conn: &duckdb::Connection, current: &str) -> Result<Vec<String>, String> {
    conn.prepare(
        "SELECT target_month FROM month_pulls WHERE target_month >= ?
         UNION
         SELECT target_month FROM proposals WHERE target_month >= ?
         ORDER BY 1",
    )
    .map_err(err)?
    .query_map(duckdb::params![current, current], |r| r.get(0))
    .map_err(err)?
    .collect::<Result<_, _>>()
    .map_err(err)
}

/// Pause between the refresh's GETs (roster, groups, one calendar per
/// month) — a handful of calls, but Sling's limit is ~20/min.
const REFRESH_GET_DELAY_SECS: u64 = 1;

#[tauri::command(async)]
pub fn refresh_availability_from_sling(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
) -> Result<AvailabilityRefreshResult, String> {
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — log in to Sling first".to_string())?
    };
    let current = chrono::Local::now().format("%Y-%m").to_string();
    let (cfg, months) = {
        let conn = db.0.lock().map_err(err)?;
        (load_studio_config_checked(&conn)?, refresh_months(&conn, &current)?)
    };
    if months.is_empty() {
        return Err("Nothing to refresh — pull a month from Sling first.".to_string());
    }

    // Network first (no DB lock held), then one transaction for all writes:
    // a failure part-way leaves the previous data intact.
    let pause = || std::thread::sleep(std::time::Duration::from_secs(REFRESH_GET_DELAY_SECS));
    let users = sling::fetch_users(&token_str).map_err(err)?;
    pause();
    let groups = sling::fetch_groups(&token_str).map_err(err)?;
    let mut calendars = Vec::with_capacity(months.len());
    for m in &months {
        pause();
        calendars.push((m.clone(), sling::fetch_calendar(&token_str, &cfg, m).map_err(err)?));
    }

    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;
    let roster = sync_roster(&tx, &users, &groups, &cfg)?;
    let roster_ids: std::collections::HashSet<i32> = {
        let mut s = tx.prepare("SELECT sling_user_id FROM teachers WHERE active = TRUE").map_err(err)?;
        s.query_map([], |r| r.get(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
    };
    let mut out = Vec::with_capacity(calendars.len());
    for (month, events) in &calendars {
        let (availability_count, external_shift_count) =
            write_month_events(&tx, month, events, &roster_ids, &cfg)?;
        // The month's data is now as fresh as a full pull — drafts generated
        // or checked before this are stale until re-checked.
        tx.execute(
            "INSERT OR REPLACE INTO month_pulls
                (target_month, pulled_at, user_count, qual_count, availability_count, external_shift_count)
             VALUES (?, now(), ?, ?, ?, ?)",
            duckdb::params![month, roster_ids.len() as i64, roster.qualifications, availability_count, external_shift_count],
        )
        .map_err(err)?;
        out.push(MonthRefresh { target_month: month.clone(), availability_count, external_shift_count });
    }
    tx.commit().map_err(err)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(AvailabilityRefreshResult { months: out, roster, refreshed_at: chrono::Utc::now().to_rfc3339() })
}

// ============================================================
// Algorithm versions (rules-as-data + code drafts) — thin wrappers over
// src-tauri/src/algorithm.rs
// ============================================================

#[tauri::command]
pub fn list_algorithm_versions(
    app: tauri::AppHandle,
    db: State<'_, Db>,
) -> Result<Vec<crate::algorithm::AlgorithmVersion>, String> {
    let shipped = find_project_root(&app)
        .ok()
        .and_then(|root| crate::algorithm::shipped_script_sha(&root));
    let conn = db.0.lock().map_err(err)?;
    let dir = crate::algorithm::algorithms_dir(&app)?;
    crate::algorithm::list_versions(&conn, &dir, shipped.as_deref())
}

/// Adopt a version and make it active. Rules are re-validated against the
/// DB (teacher ids, class names); a rules-only adoption keeps the active
/// version's script (algorithm::adopt_version).
#[tauri::command(async)]
pub fn adopt_algorithm_version(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    description: String,
    rules: serde_json::Value,
    script_content: Option<String>,
    claude_run_id: Option<i64>,
) -> Result<i32, String> {
    let shipped = find_project_root(&app)
        .ok()
        .and_then(|root| crate::algorithm::shipped_script_sha(&root));
    let conn = db.0.lock().map_err(err)?;
    let ctx = crate::algorithm::load_rule_context(&conn)?;
    crate::algorithm::validate_rules_in_context(&rules, &ctx)?;
    let dir = crate::algorithm::algorithms_dir(&app)?;
    let v = crate::algorithm::adopt_version(
        &conn,
        &dir,
        &description,
        &rules,
        script_content.map(|s| s.replace("\r\n", "\n")).as_deref(),
        claude_run_id,
        shipped.as_deref(),
    )?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(v)
}

/// Roll back / forward: make an existing version (or 9 = the shipped
/// baseline) the one generate_proposal runs. No algorithm_versions row is
/// touched — the pointer lives in app_settings.
#[tauri::command]
pub fn set_active_algorithm_version(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    version: i32,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    let dir = crate::algorithm::algorithms_dir(&app)?;
    crate::algorithm::set_active_version(&conn, &dir, version)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(())
}

/// Delete a non-active version's script file (from algorithms/ and
/// archive/). Proposal history is untouched — the version just can't be
/// re-run any more.
#[tauri::command]
pub fn delete_algorithm_script(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    version: i32,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    let active = crate::algorithm::active_version(&conn)?;
    if active.as_ref().map(|v| v.version) == Some(version) {
        return Err("cannot delete the active version's script".to_string());
    }
    let file: Option<String> = conn
        .query_row(
            "SELECT script_file FROM algorithm_versions WHERE version = ?",
            duckdb::params![version],
            |r| r.get(0),
        )
        .map_err(|e| format!("version v{version} not found: {e:#}"))?;
    let Some(file) = file else {
        return Err("that version runs the baseline script — nothing to delete".to_string());
    };
    // Rules-only versions reuse their predecessor's script file.
    if let Some(a) = active.as_ref().filter(|a| a.script_file.as_deref() == Some(file.as_str())) {
        return Err(format!(
            "{file} is also the active version v{}'s script — make another version active first",
            a.version
        ));
    }
    let dir = crate::algorithm::algorithms_dir(&app)?;
    let mut removed = false;
    for candidate in [dir.join(&file), dir.join("archive").join(&file)] {
        if candidate.exists() {
            std::fs::remove_file(&candidate).map_err(err)?;
            removed = true;
        }
    }
    if !removed {
        return Err(format!("script {file} is already gone"));
    }
    Ok(())
}

// ============================================================
// helpers
// ============================================================

/// Directory that contains the shipped `scripts/` and `prompts/` folders.
///
/// Dev (`tauri dev`, debug build): the repo root, known at compile time from
/// CARGO_MANIFEST_DIR (src-tauri/..). This no longer depends on the process's
/// working directory.
///
/// Installed (release build): Tauri's resource dir, where `bundle.resources`
/// in tauri.conf.json copies scripts/propose.py and prompts/*.md.
fn find_project_root(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    if cfg!(debug_assertions) {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        return Ok(manifest.parent().unwrap_or(manifest).to_path_buf());
    }
    use tauri::Manager;
    let dir = app.path().resource_dir()?;
    if dir.join("scripts").join("propose.py").exists() {
        Ok(dir)
    } else {
        anyhow::bail!(
            "bundled scripts not found in {} — reinstall Barrekeep",
            dir.display()
        )
    }
}

/// Working directory for the Python process. The install folder
/// (under Program Files) is read-only, and propose.py writes
/// data/output/proposed.csv relative to its cwd, so installed builds run it
/// from the per-user app-data dir instead. Dev builds keep the repo root so
/// fixture-relative paths still resolve.
fn script_workdir(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    if cfg!(debug_assertions) {
        return find_project_root(app);
    }
    use tauri::Manager;
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Last N lines of `text`, joined with newlines. Used to keep stderr blurbs
/// short when surfacing them to the user.
fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sling::{SlingGroup, SlingUser, StudioConfig};

    fn conn_with_schema() -> duckdb::Connection {
        let conn = duckdb::Connection::open_in_memory().expect("open");
        crate::migrations::run(&conn).expect("migrations");
        conn
    }

    fn cfg() -> StudioConfig {
        StudioConfig { org_id: 41822, acting_user_id: 1930001, home_location_id: 901 }
    }

    fn groups() -> Vec<SlingGroup> {
        vec![
            SlingGroup { id: 101, name: "Classic".into(), kind: "position".into() },
            SlingGroup { id: 102, name: "Empower".into(), kind: "position".into() },
            SlingGroup { id: 901, name: "Downtown Studio".into(), kind: "location".into() },
        ]
    }

    fn user(id: i64, name: &str, group_ids: Vec<i64>) -> SlingUser {
        SlingUser { id, name: name.into(), lastname: "T".into(), active: true, group_ids }
    }

    #[test]
    fn claude_model_setting_roundtrip_and_fallback() {
        let conn = conn_with_schema();
        assert_eq!(claude_model(&conn), "claude-opus-5-5"); // unset -> default
        conn.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-haiku-4-5')", []).unwrap();
        assert_eq!(claude_model(&conn), "claude-haiku-4-5");
        conn.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-sonnet-5-5')", []).unwrap();
        assert_eq!(claude_model(&conn), "claude-sonnet-5-5");
        conn.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-9000')", []).unwrap();
        assert_eq!(claude_model(&conn), "claude-opus-5-5"); // unknown -> default
        // A setting saved by an older build (retired id) falls back too.
        conn.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-opus-4-8')", []).unwrap();
        assert_eq!(claude_model(&conn), "claude-opus-5-5");
    }

    #[test]
    fn edit_position_recomputes_end_time_and_audits() {
        let mut conn = conn_with_schema();
        conn.execute_batch(
            "INSERT INTO positions (sling_position_id, class_name, duration_minutes) VALUES
               (29470407, 'Classic', 50), (29470408, 'Empower', 45);
             INSERT INTO positions (sling_position_id, class_name, duration_minutes, active)
               VALUES (29470409, 'Retired', 30, FALSE);
             INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max)
               VALUES (1930001, 'Alex', 4, 5);
             INSERT INTO proposals (target_month, algorithm_version, parameters)
               VALUES ('2026-08', 'v9', '{}');
             INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time,
                 sling_position_id, sling_user_id, generation_reason)
             SELECT id, DATE '2026-08-03', '09:00', '09:50', 29470407, 1930001, 'test'
             FROM proposals;",
        )
        .unwrap();
        let sid: i64 = conn
            .query_row("SELECT min(id) FROM proposal_shifts", [], |r| r.get(0))
            .unwrap();

        edit_position_impl(&mut conn, sid, 29470408, Some("format swap".into())).expect("edit ok");
        let (pid, end): (i32, String) = conn
            .query_row(
                "SELECT sling_position_id, end_time FROM proposal_shifts WHERE id = ?",
                duckdb::params![sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(pid, 29470408);
        assert_eq!(end, "09:45"); // 09:00 + 45min
        let (field, old_v, new_v): (String, String, String) = conn
            .query_row(
                "SELECT field, old_value, new_value FROM edits
                 WHERE proposal_shift_id = ? ORDER BY id DESC LIMIT 1",
                duckdb::params![sid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(field, "sling_position_id");
        assert_eq!((old_v.as_str(), new_v.as_str()), ("29470407", "29470408"));

        // Guards: unchanged position, inactive position.
        assert!(edit_position_impl(&mut conn, sid, 29470408, None).is_err());
        assert!(edit_position_impl(&mut conn, sid, 29470409, None).is_err());
    }

    #[test]
    fn validate_claude_edits_marks_bad_edits() {
        let conn = conn_with_schema();
        conn.execute_batch(
            "INSERT INTO positions (sling_position_id, class_name, duration_minutes) VALUES
               (101, 'Classic', 50), (102, 'Empower', 45);
             INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max)
               VALUES (501, 'Alex', 4, 5), (502, 'Kay', 4, 5);
             INSERT INTO proposals (target_month, algorithm_version, parameters)
               VALUES ('2026-08', 'v9', '{}');
             INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time,
                 sling_position_id, sling_user_id, generation_reason)
             SELECT id, DATE '2026-08-03', '09:00', '09:50', 101, 501, 'test' FROM proposals;",
        )
        .unwrap();
        let pid: i64 = conn.query_row("SELECT min(id) FROM proposals", [], |r| r.get(0)).unwrap();
        let sid: i64 = conn.query_row("SELECT min(id) FROM proposal_shifts", [], |r| r.get(0)).unwrap();

        let mk = |shift, action: &str, uid: Option<i32>, class: Option<&str>| {
            crate::editor::ProposedEdit {
                proposal_shift_id: shift,
                action: action.to_string(),
                new_user_id: uid,
                new_class_name: class.map(String::from),
                rationale: "t".into(),
                valid: true,
                validation_note: None,
            }
        };
        let mut edits = vec![
            mk(sid, "reassign", Some(502), None),        // ok
            mk(sid, "reassign", Some(501), None),        // same teacher
            mk(sid, "reassign", Some(999), None),        // unknown teacher
            mk(sid, "change_format", None, Some("Empower")), // ok
            mk(sid, "change_format", None, Some("Yoga")),    // unknown class
            mk(sid, "unassign", None, None),             // ok
            mk(9999, "unassign", None, None),            // unknown slot
            mk(sid, "explode", None, None),              // unknown action
        ];
        validate_claude_edits(&conn, pid, &mut edits).unwrap();
        let flags: Vec<bool> = edits.iter().map(|e| e.valid).collect();
        assert_eq!(flags, vec![true, false, false, true, false, true, false, false]);
        assert!(edits[6].validation_note.as_deref().unwrap().contains("not in this proposal"));
    }

    fn rs(d: &str, t: &str, p: i32, class: &str, u: Option<i32>) -> RunShift {
        RunShift {
            date: d.to_string(),
            start: t.to_string(),
            position_id: p,
            class_name: class.to_string(),
            user_id: u,
            coteach_label: String::new(),
            dropped: u.is_none(),
        }
    }

    fn names() -> std::collections::HashMap<i32, String> {
        [(501, "Alex".to_string()), (502, "Kay".to_string()), (503, "Cee".to_string())]
            .into_iter()
            .collect()
    }

    fn no_moves() -> std::collections::HashMap<&'static str, std::collections::HashSet<String>> {
        Default::default()
    }

    #[test]
    fn compare_runs_counts_and_labels() {
        let base = vec![
            rs("2026-08-03", "09:00", 101, "Classic", Some(501)),
            rs("2026-08-03", "17:30", 102, "Empower", Some(502)),
            rs("2026-08-04", "09:00", 101, "Classic", Some(503)),
            rs("2026-08-05", "09:00", 101, "Classic", Some(503)),
        ];
        let same = compare_runs("2026-08", &base, &base, &names(), &no_moves());
        assert_eq!((same.status.as_str(), same.changed_count, same.changes.len()), ("pass", 0, 0));

        // One teacher swap (25% — at the threshold, still passes).
        let mut swapped = base.clone();
        swapped[0].user_id = Some(502);
        let v = compare_runs("2026-08", &base, &swapped, &names(), &no_moves());
        assert_eq!((v.status.as_str(), v.changed_count), ("pass", 1));
        let c = &v.changes[0];
        assert_eq!(
            (c.kind.as_str(), c.weekday.as_str(), c.teacher_before.as_deref(), c.teacher_after.as_deref()),
            ("changed", "Mon", Some("Alex"), Some("Kay"))
        );

        // Two changes = 50%: needs an explicit confirm. A format flex (new
        // position at the same time) and a newly dropped slot both count as
        // changed assignments, not added/removed slots.
        let mut two = base.clone();
        two[1] = rs("2026-08-03", "17:30", 101, "Classic", Some(502));
        two[2].user_id = None;
        two[2].dropped = true;
        let v = compare_runs("2026-08", &base, &two, &names(), &no_moves());
        assert_eq!((v.status.as_str(), v.changed_count, v.added_count), ("needs_confirm", 2, 0));
        assert!(v.changes.iter().any(|c| c.class_before.as_deref() == Some("Empower")
            && c.class_after.as_deref() == Some("Classic")));
        assert!(v.changes.iter().any(|c| c.teacher_after.as_deref() == Some("Dropped")));

        // A slot that vanishes and one that appears: unexplained, so it needs
        // an explicit confirm (flagged per slot), even with 0% reassigned.
        let mut moved = base.clone();
        moved.remove(3);
        moved.push(rs("2026-08-05", "10:00", 101, "Classic", Some(503)));
        let v = compare_runs("2026-08", &base, &moved, &names(), &no_moves());
        assert_eq!(
            (v.status.as_str(), v.changed_count, v.added_count, v.removed_count, v.unexpected_count),
            ("needs_confirm", 0, 1, 1, 2)
        );
        assert!(v.reasons[0].contains("2 slot(s) appeared or disappeared"), "{:?}", v.reasons);
        assert!(v.changes.iter().all(|c| !c.expected));
    }

    #[test]
    fn compare_runs_keeps_coteach_rows_distinct() {
        let mut co = rs("2026-08-08", "10:00", 105, "Focus", Some(501));
        co.coteach_label = "Alex + Kay".into();
        let solo = rs("2026-08-08", "10:00", 101, "Classic", Some(503));
        let base = vec![co.clone(), solo.clone()];
        // Same two rows in a different order: no change.
        let v = compare_runs("2026-08", &base, &[solo.clone(), co.clone()], &names(), &no_moves());
        assert_eq!(v.changed_count, 0);
        // A different co-teacher (same primary uid) is a change, labelled by pair.
        let mut co2 = co.clone();
        co2.coteach_label = "Alex + Cee".into();
        let v = compare_runs("2026-08", &base, &[co2, solo], &names(), &no_moves());
        assert_eq!(v.changed_count, 1);
        assert_eq!(v.changes[0].teacher_before.as_deref(), Some("Alex + Kay"));
        assert_eq!(v.changes[0].teacher_after.as_deref(), Some("Alex + Cee"));
    }

    #[test]
    fn time_shift_changes_make_slot_moves_expected() {
        let active = json!({});
        let cand = json!({"sat_time_shifts": {"08:00": "08:30"}});
        let moves = expected_moves(&active, &cand);
        // 2026-08-08 is a Saturday.
        let base = vec![rs("2026-08-08", "08:00", 101, "Classic", Some(501))];
        let after = vec![rs("2026-08-08", "08:30", 101, "Classic", Some(501))];
        let v = compare_runs("2026-08", &base, &after, &names(), &moves);
        assert_eq!((v.status.as_str(), v.unexpected_count, v.added_count), ("pass", 0, 1));
        assert!(v.changes.iter().all(|c| c.expected));
        // The same move on a Sunday is not explained by a Saturday rule.
        let base = vec![rs("2026-08-09", "08:00", 101, "Classic", Some(501))];
        let after = vec![rs("2026-08-09", "08:30", 101, "Classic", Some(501))];
        let v = compare_runs("2026-08", &base, &after, &names(), &moves);
        assert_eq!((v.status.as_str(), v.unexpected_count), ("needs_confirm", 2));
    }

    /// End-to-end "reproduce last month" on the fixture payload with the real
    /// scripts/propose.py (skipped when python3 isn't available).
    #[test]
    fn validate_candidate_runs_both_scripts() {
        let python_ok = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !python_ok {
            eprintln!("python not available — skipping");
            return;
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let script = root.join("scripts/propose.py");
        let payload: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("scripts/tests/fixture_payload.json")).unwrap(),
        )
        .unwrap();
        let month = payload["target_month"].as_str().unwrap().to_string();

        let v = validate_candidate(&month, &payload, &root, &script, &json!({}), &script, &json!({}), &names());
        assert_eq!(v.status, "pass", "{:?}", v.error);
        assert!(v.slot_count > 0);
        assert_eq!(v.changed_count, 0);

        // Blocking the lead from Mon 09:00 moves every Mon 09:00 class.
        let rules = json!({"teacher_slot_blocklist": [
            {"sling_user_id": 501, "weekday": "Mon", "time": "09:00"}]});
        let v = validate_candidate(&month, &payload, &root, &script, &json!({}), &script, &rules, &names());
        assert!(v.changed_count > 0);
        assert!(v.changes.iter().all(|c| c.weekday == "Mon" && c.start == "09:00"));
        assert!(v.changes.iter().all(|c| c.teacher_before.as_deref() == Some("Alex")));

        // A broken candidate script is an error, never adoptable.
        let bad = std::env::temp_dir().join(format!("bk-bad-{}.py", std::process::id()));
        std::fs::write(&bad, "raise SystemExit('boom')\n").unwrap();
        let v = validate_candidate(&month, &payload, &root, &script, &json!({}), &bad, &json!({}), &names());
        let _ = std::fs::remove_file(&bad);
        assert_eq!(v.status, "error");
        assert!(v.error.unwrap().contains("candidate failed"));
    }

    /// The payload builder still fails loudly with no trailing history
    /// (blank-calendar guard), and stays independent of the version store.
    #[test]
    fn build_payload_requires_history() {
        let conn = conn_with_schema();
        let e = build_propose_payload(&conn, "2026-09").unwrap_err();
        assert!(e.contains("Pull from Sling"), "{e}");
    }

    /// sync_roster must be a no-op-safe delta sync: repeated runs with the
    /// same Sling data change nothing (and count nothing), user-managed
    /// position toggles survive, and the whole thing works while a proposal
    /// references the positions (the original pull-failure scenario).
    #[test]
    fn sync_roster_is_idempotent_delta_sync() {
        let mut conn = conn_with_schema();
        let users = vec![
            user(1930001, "Alex", vec![901, 101, 102]),
            user(1930002, "Kayla", vec![901, 101]),
        ];

        let tx = conn.transaction().unwrap();
        let s1 = sync_roster(&tx, &users, &groups(), &cfg()).unwrap();
        tx.commit().unwrap();
        assert_eq!(s1.teachers_active, 2);
        assert_eq!(s1.positions_active, 2);
        assert_eq!(s1.teachers_deactivated, 0);
        assert_eq!(s1.positions_deactivated, 0);

        // A generated proposal now references position 101 (this is the state
        // that used to make the next sync explode — see migration 0009).
        conn.execute_batch(
            "INSERT INTO proposals (target_month, algorithm_version, parameters, is_current)
             VALUES ('2026-08', 'v3', '{}', TRUE);
             INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time,
                 sling_position_id, sling_user_id, generation_reason)
             SELECT id, DATE '2026-08-03', '09:00', '10:00', 101, 1930001, 'rotation' FROM proposals;",
        ).unwrap();
        // The lead teacher deactivates a position she doesn't schedule.
        conn.execute("UPDATE positions SET active = FALSE WHERE sling_position_id = 102", []).unwrap();

        let tx = conn.transaction().unwrap();
        let s2 = sync_roster(&tx, &users, &groups(), &cfg()).unwrap();
        tx.commit().unwrap();
        // Idempotent: nothing (further) deactivated, manual toggle preserved.
        assert_eq!(s2.teachers_deactivated, 0);
        assert_eq!(s2.positions_deactivated, 0);
        let active_102: bool = conn.query_row(
            "SELECT active FROM positions WHERE sling_position_id = 102", [], |r| r.get(0)).unwrap();
        assert!(!active_102, "user-managed schedulable toggle must survive a sync");

        // Renames propagate; departures deactivate exactly once.
        let mut renamed = groups();
        renamed[0].name = "Classique".into();
        let departed = vec![users[0].clone()];
        let tx = conn.transaction().unwrap();
        let s3 = sync_roster(&tx, &departed, &renamed, &cfg()).unwrap();
        tx.commit().unwrap();
        assert_eq!(s3.teachers_deactivated, 1);
        let name: String = conn.query_row(
            "SELECT class_name FROM positions WHERE sling_position_id = 101", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "Classique");

        let tx = conn.transaction().unwrap();
        let s4 = sync_roster(&tx, &departed, &renamed, &cfg()).unwrap();
        tx.commit().unwrap();
        assert_eq!(s4.teachers_deactivated, 0, "already-inactive teacher must not recount");
    }

    /// Blocks are visible for every month they OVERLAP, not just the month
    /// they start in — a leave spanning a month boundary must show up for
    /// the second month too.
    #[test]
    fn availability_blocks_visible_across_month_boundary() {
        let conn = conn_with_schema();
        conn.execute_batch(
            "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max)
             VALUES (1930001, 'Alex', 4, 5);
             INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at) VALUES
               (1930001, 'leave', TIMESTAMPTZ '2026-07-25 00:00:00-05', TIMESTAMPTZ '2026-08-10 23:59:59-05'),
               (1930001, 'leave', TIMESTAMPTZ '2026-08-20 08:00:00-05', TIMESTAMPTZ '2026-08-20 12:00:00-05'),
               (1930001, 'leave', TIMESTAMPTZ '2026-06-01 00:00:00-05', TIMESTAMPTZ '2026-06-05 00:00:00-05');",
        ).unwrap();
        let aug = query_availability_blocks(&conn, "2026-08").unwrap();
        assert_eq!(aug.len(), 2, "spanning + in-month blocks visible, June-only block excluded");
        let jul = query_availability_blocks(&conn, "2026-07").unwrap();
        assert_eq!(jul.len(), 1, "spanning block also visible from its starting month");
    }
}
