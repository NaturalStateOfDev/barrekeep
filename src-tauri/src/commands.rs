// Tauri IPC commands — the surface the React frontend calls into.
// Add new commands here, then register them in lib.rs's invoke_handler!.
//
// Convention: commands return Result<T, String> so errors serialize to JS
// as plain strings (anyhow's full chain via {:#}).

use std::path::PathBuf;
use std::process::Command;
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

#[derive(Serialize)]
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

// class_name is also in the JSON payload but we don't read it here — class
// names come from a JOIN on positions in the read paths. serde silently
// ignores unknown fields, so dropping it is safe.
#[derive(Deserialize)]
struct ProposeShift {
    shift_date: String,
    start_time: String,
    end_time: String,
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
                    "dtstart": format!("{date}T{start}:00-05:00"),
                    "dtend": format!("{date}T{end}:00-05:00"),
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
                    "dtstart": format!("{date}T{start}:00-05:00"),
                    "dtend": format!("{date}T{end}:00-05:00"),
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
    project_root: &std::path::Path,
    payload_json: &serde_json::Value,
    target_month: &str,
) -> Result<(ProposeOutput, String), String> {
    use std::io::Write;
    use std::process::Stdio;

    let python_bin = if cfg!(windows) { "python" } else { "python3" };
    let script = script_path
        .to_str()
        .ok_or_else(|| "script path is not valid UTF-8".to_string())?;
    let mut child = Command::new(python_bin)
        .args([script, "--json-out", "--from-stdin", "--target-month", target_month])
        .current_dir(project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn {python_bin}: {e}"))?;

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

#[tauri::command]
pub fn generate_proposal(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    target_month: String,
) -> Result<GenerateResult, String> {
    // Step 1: build the payload and resolve the active algorithm version
    // (rules + script). cwd stays at project root so the baseline script's
    // relative paths keep resolving in dev mode.
    let project_root = find_project_root().map_err(err)?;
    let (payload_json, script_path) = {
        let conn = db.0.lock().map_err(err)?;
        let mut payload = build_propose_payload(&conn, &target_month)?;
        let active = crate::algorithm::active_version(&conn)?;
        let script = match &active {
            Some(v) => {
                let dir = crate::algorithm::algorithms_dir(&app)?;
                payload["rules"] = v.rules.clone();
                payload["version_label"] =
                    serde_json::Value::String(format!("v{}", v.version));
                crate::algorithm::resolve_script(&dir, v, &project_root)?
            }
            None => project_root.join("scripts").join("propose.py"),
        };
        (payload, script)
    };

    let (payload, stderr_tail) =
        spawn_propose(&script_path, &project_root, &payload_json, &target_month)?;

    // Step 2: write the proposal + shifts to DuckDB in a single transaction.
    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;

    // Demote any prior "current" proposal for this month.
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
}

#[tauri::command]
pub fn list_proposals(db: State<'_, Db>) -> Result<Vec<ProposalSummary>, String> {
    let conn = db.0.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT
                p.id,
                p.target_month,
                p.algorithm_version,
                CAST(p.generated_at AS VARCHAR),
                p.is_current,
                (SELECT count(*) FROM proposal_shifts ps WHERE ps.proposal_id = p.id) AS shift_count,
                (SELECT count(*) FROM proposal_shifts ps WHERE ps.proposal_id = p.id AND ps.is_dropped) AS dropped_count,
                (SELECT count(*) FROM edits e
                    JOIN proposal_shifts ps2 ON ps2.id = e.proposal_shift_id
                    WHERE ps2.proposal_id = p.id AND NOT e.reverted) AS edit_count
             FROM proposals p
             ORDER BY p.generated_at DESC",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ProposalSummary {
                id: r.get(0)?,
                target_month: r.get(1)?,
                algorithm_version: r.get(2)?,
                generated_at: r.get(3)?,
                is_current: r.get(4)?,
                shift_count: r.get(5)?,
                dropped_count: r.get(6)?,
                edit_count: r.get(7)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

#[tauri::command]
pub fn get_proposal(
    db: State<'_, Db>,
    proposal_id: i64,
) -> Result<ProposalDetail, String> {
    let conn = db.0.lock().map_err(err)?;

    let summary: ProposalSummary = conn
        .query_row(
            "SELECT
                p.id,
                p.target_month,
                p.algorithm_version,
                CAST(p.generated_at AS VARCHAR),
                p.is_current,
                (SELECT count(*) FROM proposal_shifts ps WHERE ps.proposal_id = p.id),
                (SELECT count(*) FROM proposal_shifts ps WHERE ps.proposal_id = p.id AND ps.is_dropped),
                (SELECT count(*) FROM edits e
                    JOIN proposal_shifts ps2 ON ps2.id = e.proposal_shift_id
                    WHERE ps2.proposal_id = p.id AND NOT e.reverted)
             FROM proposals p
             WHERE p.id = ?",
            duckdb::params![proposal_id],
            |r| {
                Ok(ProposalSummary {
                    id: r.get(0)?,
                    target_month: r.get(1)?,
                    algorithm_version: r.get(2)?,
                    generated_at: r.get(3)?,
                    is_current: r.get(4)?,
                    shift_count: r.get(5)?,
                    dropped_count: r.get(6)?,
                    edit_count: r.get(7)?,
                })
            },
        )
        .map_err(err)?;

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

    let (is_stale, last_pulled_at): (bool, Option<String>) = {
        let row: Result<(Option<String>, String), _> = conn.query_row(
            "SELECT
                CAST(mp.pulled_at AS VARCHAR),
                CAST(p.generated_at AS VARCHAR)
             FROM proposals p
             LEFT JOIN month_pulls mp ON mp.target_month = p.target_month
             WHERE p.id = ?",
            duckdb::params![proposal_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );
        match row {
            Ok((Some(pulled), generated)) => (pulled > generated, Some(pulled)),
            _ => (false, None),
        }
    };

    Ok(ProposalDetail { summary, shifts, is_stale, last_pulled_at })
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

/// Model allowlist for the Claude features. Exact ids only — an unknown
/// stored value falls back to the default at call time.
pub const CLAUDE_MODELS: &[&str] = &["claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5"];
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-4-8";

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

#[tauri::command]
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

#[tauri::command]
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
    let user_payload = {
        let conn = db.0.lock().map_err(err)?;

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

        json!({
            "proposal": {
                "id": proposal_id,
                "target_month": target_month,
                "algorithm_version": algorithm_version,
            },
            "shifts": shifts,
            "edits": edits,
            "roster": roster,
        })
    };

    // 3. Call Anthropic. This is the slow step (~5–30s).
    let result = review::run_review(&api_key, &user_payload).map_err(err)?;

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
            // Re-parse the stored payload. If parsing fails, return an empty
            // suggestion list rather than failing the whole query.
            let parsed: review::ReviewPayload = serde_json::from_str(&output_text)
                .unwrap_or(review::ReviewPayload {
                    suggestions: vec![],
                    overall_assessment: "(could not parse stored review)".into(),
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
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

// ============================================================
// Sling token (in-memory cache; Stronghold is the persistence layer)
// ============================================================

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
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
            "Studio not configured — set your Sling org, acting-user, and location IDs in \
             Settings → Studio configuration before pulling."
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

    let (m_start, m_end) = sling::month_range(&target_month).map_err(err)?;
    tx.execute(
        "DELETE FROM availability_blocks
         WHERE starts_at >= CAST(? AS TIMESTAMPTZ) AND starts_at <= CAST(? AS TIMESTAMPTZ)",
        duckdb::params![&m_start, &m_end],
    ).map_err(err)?;
    let mut availability_count: i64 = 0;
    for e in &payload.month_events {
        if e.kind != "availability" && e.kind != "leave" { continue; }
        // Ownership guard: this pull owns (deletes + rewrites) only blocks
        // that START in the target month — the same window the DELETE above
        // clears. A spanning block Sling returns for a later month would
        // otherwise be inserted a second time.
        if e.dtstart.get(0..7) != Some(target_month.as_str()) { continue; }
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
        duckdb::params![&target_month],
    ).map_err(err)?;
    let mut external_shift_count: i64 = 0;
    let home_location_shifts = sling::filter_events(&payload.month_events, &["shift"], cfg.home_location_id);
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
            duckdb::params![shift_id, &target_month, &date_part, &start_hm, &end_hm, uid, pid, &status],
        ).map_err(err)?;
        external_shift_count += 1;
    }
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
// Push proposal to Sling — dry-run (preview) command
// ============================================================

#[derive(serde::Serialize)]
pub struct PushPreviewItem {
    pub date: String,
    pub start: String,
    pub end: String,
    pub class_name: String,
    pub teacher_name: String,
}

#[derive(serde::Serialize)]
pub struct PushPreview {
    pub total: i64,
    pub skipped_count: i64,
    pub to_create: Vec<PushPreviewItem>,
}

#[derive(serde::Serialize, Clone)]
pub struct PushSummary {
    pub push_id: i64,
    pub created: i64,
    pub failed: i64,
    pub skipped: i64,
}

#[derive(serde::Serialize, Clone)]
pub struct PushProgress {
    pub total: i64,
    pub done: i64,
    pub created: i64,
    pub failed: i64,
    pub skipped: i64,
    pub last_label: String,
    pub last_outcome: String,
}

/// Load proposal rows + roster map + studio config, then build the gated
/// push specs and the target month string. Shared by dry-run and execute.
fn build_specs_for_proposal(
    conn: &duckdb::Connection,
    proposal_id: i64,
) -> Result<(Vec<crate::sling::PushSpec>, crate::sling::StudioConfig, String), String> {
    let studio_cfg = load_studio_config(conn)?;
    if studio_cfg.org_id == 0 || studio_cfg.home_location_id == 0 {
        return Err(
            "Studio not configured — set your Sling org, acting-user, and location IDs in \
             Settings → Studio configuration before pushing."
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
                teacher_name: r.get(6)?,
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
pub fn push_proposal_dry_run(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
) -> Result<PushPreview, String> {
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone()
            .ok_or_else(|| "no Sling token — paste one in Settings".to_string())?
    };
    let (specs, cfg, month) = {
        let conn = db.0.lock().map_err(err)?;
        build_specs_for_proposal(&conn, proposal_id)?
    };
    let events =
        crate::sling::fetch_calendar(&token_str, &cfg, &month).map_err(err)?;
    let existing =
        crate::sling::existing_fingerprints(&events, cfg.home_location_id);

    let total = specs.len() as i64;
    let mut to_create = Vec::new();
    let mut skipped_count = 0i64;
    for s in &specs {
        if existing.contains(&crate::sling::spec_fingerprint(s, cfg.home_location_id)) {
            skipped_count += 1;
        } else {
            to_create.push(PushPreviewItem {
                date: s.date.clone(),
                start: s.start.clone(),
                end: s.end.clone(),
                class_name: s.class_name.clone(),
                teacher_name: s.teacher_name.clone(),
            });
        }
    }
    Ok(PushPreview {
        total,
        skipped_count,
        to_create,
    })
}

const PUSH_BATCH_SIZE: usize = 10;
const PUSH_INTRA_DELAY_SECS: u64 = 1;
const PUSH_INTER_DELAY_SECS: u64 = 10;

#[tauri::command]
pub fn push_proposal_execute(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
) -> Result<PushSummary, String> {
    use tauri::Emitter;

    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — paste one in Settings".to_string())?
    };
    let (specs, cfg, month) = {
        let conn = db.0.lock().map_err(err)?;
        build_specs_for_proposal(&conn, proposal_id)?
    };
    let (viewdates, cachedates) = crate::sling::view_cache_dates(&month).map_err(err)?;

    // Re-dedupe at execute time (idempotent re-push: only POST what's missing).
    let events = crate::sling::fetch_calendar(&token_str, &cfg, &month).map_err(err)?;
    let existing = crate::sling::existing_fingerprints(&events, cfg.home_location_id);
    let to_create: Vec<&crate::sling::PushSpec> = specs.iter()
        .filter(|s| !existing.contains(&crate::sling::spec_fingerprint(s, cfg.home_location_id)))
        .collect();
    let skipped = (specs.len() - to_create.len()) as i64;
    let total = to_create.len() as i64;

    // Open the audit row.
    let push_id: i64 = {
        let conn = db.0.lock().map_err(err)?;
        conn.query_row(
            "INSERT INTO pushes (proposal_id, shifts_attempted, shifts_skipped) VALUES (?, ?, ?) RETURNING id",
            duckdb::params![proposal_id, total, skipped],
            |r| r.get(0),
        ).map_err(err)?
    };

    let mut created = 0i64;
    let mut failed = 0i64;
    let mut aborted_401 = false;

    'outer: for (idx, chunk) in to_create.chunks(PUSH_BATCH_SIZE).enumerate() {
        for (j, s) in chunk.iter().enumerate() {
            let label = format!("{} {} {} → {}", s.date, s.start, s.class_name, s.teacher_name);
            let (outcome, sling_id, errmsg): (&str, Option<String>, Option<String>) =
                match crate::sling::push_shift(&token_str, &cfg, s, &viewdates, &cachedates) {
                    Ok(id) => { created += 1; ("created", Some(id.to_string()), None) }
                    Err(e) if e.to_string() == "sling-401" => { aborted_401 = true; failed += 1; ("failed", None, Some("token expired".into())) }
                    Err(e) => { failed += 1; ("failed", None, Some(e.to_string())) }
                };
            {
                let conn = db.0.lock().map_err(err)?;
                conn.execute(
                    "INSERT INTO push_results (push_id, proposal_shift_id, outcome, sling_shift_id, error_message)
                     VALUES (?, ?, ?, ?, ?)",
                    duckdb::params![push_id, s.proposal_shift_id, outcome, sling_id, errmsg],
                ).map_err(err)?;
            }
            let done = created + failed;
            let _ = app.emit("push-progress", PushProgress {
                total, done, created, failed, skipped,
                last_label: label, last_outcome: outcome.to_string(),
            });
            if aborted_401 { break 'outer; }
            if j < chunk.len() - 1 {
                std::thread::sleep(std::time::Duration::from_secs(PUSH_INTRA_DELAY_SECS));
            }
        }
        if idx < to_create.len().div_ceil(PUSH_BATCH_SIZE) - 1 {
            std::thread::sleep(std::time::Duration::from_secs(PUSH_INTER_DELAY_SECS));
        }
    }

    // Close the audit row.
    {
        let conn = db.0.lock().map_err(err)?;
        conn.execute(
            "UPDATE pushes SET finished_at = now(), shifts_succeeded = ?, shifts_failed = ? WHERE id = ?",
            duckdb::params![created, failed, push_id],
        ).map_err(err)?;
    }

    if aborted_401 {
        return Err(format!("sling-401: token expired after creating {created} shift(s)"));
    }
    Ok(PushSummary { push_id, created, failed, skipped })
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
pub fn open_sling_login_window(app: tauri::AppHandle) -> Result<(), String> {
    // Build the login webview on the MAIN thread. Tauri runs sync commands on a
    // worker thread; creating a WebView2 window off the main thread deadlocks the
    // event-loop message pump on Windows — the window frame appears but its
    // content never initializes and the whole app freezes (the window won't even
    // close). WebKitGTK tolerates off-thread creation, so this only bit on
    // Windows. run_on_main_thread queues the build onto the event loop, where
    // webview creation is valid; the command returns immediately.
    let app_for_build = app.clone();
    app.run_on_main_thread(move || {
        if let Err(e) = crate::sling_login::open_login_window(app_for_build) {
            eprintln!("[sling_login] failed to open login window: {e}");
        }
    })
    .map_err(err)
}

#[tauri::command]
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

// ============================================================
// Standalone roster refresh — sync roster without pulling a month
// ============================================================

#[tauri::command]
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
        return Err("Studio not configured — set your Sling org, acting-user, and location IDs in \
                    Settings → Studio configuration before refreshing the roster.".to_string());
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
// Algorithm versions (rules-as-data + code drafts) — thin wrappers over
// src-tauri/src/algorithm.rs
// ============================================================

#[tauri::command]
pub fn list_algorithm_versions(
    app: tauri::AppHandle,
    db: State<'_, Db>,
) -> Result<Vec<crate::algorithm::AlgorithmVersion>, String> {
    let conn = db.0.lock().map_err(err)?;
    let dir = crate::algorithm::algorithms_dir(&app)?;
    crate::algorithm::list_versions(&conn, &dir)
}

#[tauri::command]
pub fn adopt_algorithm_version(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    description: String,
    rules: serde_json::Value,
    script_content: Option<String>,
    claude_run_id: Option<i64>,
) -> Result<i32, String> {
    let conn = db.0.lock().map_err(err)?;
    let dir = crate::algorithm::algorithms_dir(&app)?;
    let v = crate::algorithm::adopt_version(
        &conn,
        &dir,
        &description,
        &rules,
        script_content.as_deref(),
        claude_run_id,
    )?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(v)
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
    let active = crate::algorithm::active_version(&conn)?
        .map(|v| v.version)
        .unwrap_or(crate::algorithm::BASELINE_VERSION);
    if version == active {
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

/// Walk up from the current working directory until we find package.json.
/// Used to launch python sidecars from the project root no matter where
/// Tauri's binary was invoked from.
fn find_project_root() -> anyhow::Result<PathBuf> {
    let mut cwd = std::env::current_dir()?;
    loop {
        if cwd.join("package.json").exists() {
            return Ok(cwd);
        }
        if !cwd.pop() {
            anyhow::bail!(
                "could not find project root — no package.json found walking up from cwd"
            );
        }
    }
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
        assert_eq!(claude_model(&conn), "claude-opus-4-8"); // unset -> default
        conn.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-haiku-4-5')", []).unwrap();
        assert_eq!(claude_model(&conn), "claude-haiku-4-5");
        conn.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-9000')", []).unwrap();
        assert_eq!(claude_model(&conn), "claude-opus-4-8"); // unknown -> default
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
