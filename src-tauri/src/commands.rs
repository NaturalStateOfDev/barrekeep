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
pub struct SlingCandidate {
    pub sling_user_id: i32,
    pub display_name: String,
    pub active: bool,
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
    pub new_users: Vec<NewUserSummary>,
}

#[derive(Serialize)]
pub struct NewUserSummary {
    pub sling_user_id: i32,
    pub display_name: String,
    pub active: bool,
    pub locations: Option<String>,
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
pub fn list_sling_candidates(db: State<'_, Db>) -> Result<Vec<SlingCandidate>, String> {
    let conn = db.0.lock().map_err(err)?;
    let mut stmt = conn
        .prepare(
            "SELECT sling_user_id, display_name, active, locations
             FROM sling_candidates
             ORDER BY display_name",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SlingCandidate {
                sling_user_id: r.get(0)?,
                display_name: r.get(1)?,
                active: r.get(2)?,
                locations: r.get(3)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
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

#[tauri::command]
pub fn generate_proposal(
    db: State<'_, Db>,
    target_month: String,
) -> Result<GenerateResult, String> {
    // Step 1: spawn propose.py with cwd at project root (so its relative
    // fixture paths resolve correctly). Find the project root by walking up
    // from the current dir looking for package.json — works in dev mode.
    let project_root = find_project_root().map_err(err)?;
    let python_bin = if cfg!(windows) { "python" } else { "python3" };

    // Build the input payload the ported propose.py expects on stdin.
    let payload_json = {
        let conn = db.0.lock().map_err(err)?;

        // Studio's home location id — propose.py filters shifts to this.
        let studio_cfg = load_studio_config(&conn)?;

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
            stmt.query_map(duckdb::params![&cutoff, &target_month], |r| {
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

        // Month events: availability + leave + existing shifts for the target month
        let month_events: Vec<serde_json::Value> = {
            let (m_start, m_end) = crate::sling::month_range(&target_month).map_err(err)?;
            let mut stmt = conn.prepare(
                "SELECT sling_user_id, source, CAST(starts_at AS VARCHAR), CAST(ends_at AS VARCHAR)
                 FROM availability_blocks
                 WHERE starts_at >= CAST(? AS TIMESTAMPTZ) AND starts_at <= CAST(? AS TIMESTAMPTZ)"
            ).map_err(err)?;
            let mut events: Vec<serde_json::Value> = stmt.query_map(
                duckdb::params![&m_start, &m_end],
                |r| {
                    let uid: i32 = r.get(0)?;
                    let src: String = r.get(1)?;
                    let st: String = r.get(2)?;
                    let en: String = r.get(3)?;
                    Ok(serde_json::json!({
                        "type": src,
                        "dtstart": st,
                        "dtend": en,
                        "user": {"id": uid},
                    }))
                }
            ).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
            // Append target-month external shifts
            let mut stmt2 = conn.prepare(
                "SELECT CAST(shift_date AS VARCHAR), start_time, end_time, sling_user_id, sling_position_id
                 FROM external_sling_shifts
                 WHERE target_month = ?"
            ).map_err(err)?;
            for row in stmt2.query_map(duckdb::params![&target_month], |r| {
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

    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(python_bin)
        .args(["scripts/propose.py", "--json-out", "--from-stdin",
               "--target-month", &target_month])
        .current_dir(&project_root)
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
        .map_err(|e| format!("failed to wait on propose.py: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "propose.py exited {}:\n{}",
            output.status,
            tail(&stderr, 40)
        ));
    }

    let payload: ProposeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("invalid JSON from propose.py: {e}"))?;

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
        stderr_tail: tail(&String::from_utf8_lossy(&output.stderr), 20),
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
                e.reason,
                CAST(e.edited_at AS VARCHAR),
                e.reverted
             FROM edits e
             JOIN proposal_shifts ps ON ps.id = e.proposal_shift_id
             JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
             LEFT JOIN teachers t_old
                ON CAST(t_old.sling_user_id AS VARCHAR) = e.old_value
             LEFT JOIN teachers t_new
                ON CAST(t_new.sling_user_id AS VARCHAR) = e.new_value
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
                reason: r.get(10)?,
                edited_at: r.get(11)?,
                reverted: r.get(12)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

// ============================================================
// Anthropic key management
// ============================================================

#[tauri::command]
pub fn set_anthropic_key(key: State<'_, AnthropicKey>, value: String) -> Result<(), String> {
    let trimmed = value.trim();
    let mut guard = key.0.lock().map_err(err)?;
    if trimmed.is_empty() {
        *guard = None;
    } else {
        *guard = Some(trimmed.to_string());
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

    let position_group_ids = sling::position_group_ids(&payload.groups);
    let location_name_by_id = sling::location_name_by_id(&payload.groups);

    // User IDs that show up as the assignee on at least one home location shift in
    // the trailing 3 months. A user being tagged with home location + a teaching
    // position is not enough — GMs and sister-studio managers sometimes hold
    // both tags. Requiring actual recent home location shifts cuts those out.
    let home_teacher_uids: std::collections::HashSet<i32> = payload
        .history_shifts
        .iter()
        .filter_map(|s| {
            s.user
                .as_ref()
                .or_else(|| s.users.as_ref().and_then(|v| v.first()))
                .map(|u| u.id as i32)
        })
        .collect();

    let known_user_ids: std::collections::HashSet<i32> = {
        let mut stmt = tx.prepare("SELECT sling_user_id FROM teachers").map_err(err)?;
        stmt.query_map([], |r| r.get::<_, i32>(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
    };
    let known_position_ids: std::collections::HashSet<i32> = {
        let mut stmt = tx.prepare("SELECT sling_position_id FROM positions").map_err(err)?;
        stmt.query_map([], |r| r.get::<_, i32>(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
    };

    // Wipe the candidates table; we refill it below with the filtered
    // delta of "could be added to our roster" home location teachers.
    tx.execute("DELETE FROM sling_candidates", []).map_err(err)?;

    let mut user_count: i64 = 0;
    let mut new_users: Vec<NewUserSummary> = Vec::new();
    for u in &payload.users {
        let display = format!("{} {}", u.name, u.lastname).trim().to_string();
        let uid = u.id as i32;
        let user_locations = sling::compute_locations(&u.group_ids, &location_name_by_id);
        if known_user_ids.contains(&uid) {
            tx.execute(
                "UPDATE teachers SET display_name = ?, active = ?, locations = ? \
                 WHERE sling_user_id = ?",
                duckdb::params![display, u.active, user_locations, uid],
            ).map_err(err)?;
            user_count += 1;
        } else {
            // Only flag (and persist as a candidate) when the user (a) holds
            // a teaching position, (b) is active, and (c) is tagged to the
            // home location location. Drops GM/sales/front-desk and sister-studio
            // staff that share the same Sling org.
            let teaches = u.group_ids.iter().any(|g| position_group_ids.contains(g));
            let at_home = u.group_ids.contains(&cfg.home_location_id);
            if teaches && u.active && at_home {
                // Persist to the Teachers-page picker regardless — that view
                // is for browsing, occasional adds, and includes brand-new
                // hires who haven't taught yet.
                tx.execute(
                    "INSERT INTO sling_candidates (sling_user_id, display_name, active, locations) \
                     VALUES (?, ?, ?, ?)",
                    duckdb::params![uid, display.clone(), u.active, user_locations.clone()],
                ).map_err(err)?;
                // Top-of-mind alert only if they've actually taught at
                // home location in the trailing 3 months. Drops admins and
                // cross-studio managers who hold the tags but don't teach.
                if home_teacher_uids.contains(&uid) {
                    new_users.push(NewUserSummary {
                        sling_user_id: uid,
                        display_name: display,
                        active: u.active,
                        locations: user_locations,
                    });
                }
            }
        }
    }

    let mut sling_pairs: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    for u in &payload.users {
        let uid = u.id as i32;
        if !known_user_ids.contains(&uid) { continue; }
        for gid in &u.group_ids {
            if position_group_ids.contains(gid) {
                let pid = *gid as i32;
                if known_position_ids.contains(&pid) {
                    sling_pairs.insert((uid, pid));
                }
            }
        }
    }
    let existing_pairs: Vec<(i32, i32, bool)> = {
        let mut stmt = tx.prepare(
            "SELECT sling_user_id, sling_position_id, is_blocklisted FROM teacher_qualifications"
        ).map_err(err)?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
    };
    let mut qual_count: i64 = 0;
    for (uid, pid, blocked) in &existing_pairs {
        if *blocked { continue; }
        if !sling_pairs.contains(&(*uid, *pid)) {
            tx.execute(
                "DELETE FROM teacher_qualifications WHERE sling_user_id = ? AND sling_position_id = ?",
                duckdb::params![uid, pid],
            ).map_err(err)?;
        }
    }
    for (uid, pid) in &sling_pairs {
        tx.execute(
            "INSERT INTO teacher_qualifications (sling_user_id, sling_position_id)
             VALUES (?, ?) ON CONFLICT DO NOTHING",
            duckdb::params![uid, pid],
        ).map_err(err)?;
        qual_count += 1;
    }

    let (m_start, m_end) = sling::month_range(&target_month).map_err(err)?;
    tx.execute(
        "DELETE FROM availability_blocks
         WHERE starts_at >= CAST(? AS TIMESTAMPTZ) AND starts_at <= CAST(? AS TIMESTAMPTZ)",
        duckdb::params![&m_start, &m_end],
    ).map_err(err)?;
    let mut availability_count: i64 = 0;
    for e in &payload.month_events {
        if e.kind != "availability" && e.kind != "leave" { continue; }
        let uid = match e.user.as_ref().or_else(|| e.users.as_ref().and_then(|v| v.first())) {
            Some(u) => u.id as i32,
            None => continue,
        };
        if !known_user_ids.contains(&uid) { continue; }
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

    Ok(PullResult {
        target_month: target_month.clone(),
        pulled_at: chrono::Utc::now().to_rfc3339(),
        user_count,
        qual_count,
        availability_count,
        external_shift_count,
        history_shift_count,
        new_users,
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
    Ok(())
}

#[derive(Serialize)]
pub struct AvailabilityBlockRow {
    pub sling_user_id: i32,
    pub source: String,
    pub starts_at: String,
    pub ends_at: String,
}

#[tauri::command]
pub fn list_availability_blocks(
    db: State<'_, Db>,
    target_month: String,
) -> Result<Vec<AvailabilityBlockRow>, String> {
    let conn = db.0.lock().map_err(err)?;
    let (start, end) = crate::sling::month_range(&target_month).map_err(err)?;
    let mut stmt = conn.prepare(
        "SELECT sling_user_id, source, CAST(starts_at AS VARCHAR), CAST(ends_at AS VARCHAR)
         FROM availability_blocks
         WHERE starts_at >= CAST(? AS TIMESTAMPTZ) AND starts_at <= CAST(? AS TIMESTAMPTZ)"
    ).map_err(err)?;
    let rows = stmt.query_map(duckdb::params![&start, &end], |r| {
        Ok(AvailabilityBlockRow {
            sling_user_id: r.get(0)?,
            source: r.get(1)?,
            starts_at: r.get(2)?,
            ends_at: r.get(3)?,
        })
    }).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
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
// Add teacher discovered during Sling pull
// ============================================================

#[derive(Deserialize)]
pub struct AddTeacherInput {
    pub sling_user_id: i32,
    pub display_name: String,
    pub weekly_target: i32,
    pub weekly_max: i32,
    pub is_lead: bool,
}

#[tauri::command]
pub fn add_teacher_from_pull(
    db: State<'_, Db>,
    input: AddTeacherInput,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    conn.execute(
        "INSERT INTO teachers
            (sling_user_id, display_name, weekly_target, weekly_max, is_lead, variety_multiplier)
         VALUES (?, ?, ?, ?, ?, 1.0)
         ON CONFLICT (sling_user_id) DO UPDATE SET display_name = EXCLUDED.display_name",
        duckdb::params![
            input.sling_user_id, input.display_name,
            input.weekly_target, input.weekly_max, input.is_lead
        ],
    ).map_err(err)?;
    Ok(())
}

// ============================================================
// Sling browser login flow
// ============================================================

#[tauri::command]
pub fn open_sling_login_window(app: tauri::AppHandle) -> Result<(), String> {
    crate::sling_login::open_login_window(app).map_err(err)
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
