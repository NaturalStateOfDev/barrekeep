//! Versioned algorithm store (spec: docs/superpowers/specs/
//! 2026-07-06-claude-proposal-editor-design.md).
//!
//! One version sequence: v9 is the implicit baseline (the shipped
//! scripts/propose.py with empty rules); adopted versions start at 10 and
//! live as append-only rows in `algorithm_versions`. Each row carries a
//! FULL rules snapshot and optionally a script file under
//! `<app_local_data>/algorithms/` (NULL = baseline script). Rows are only
//! ever inserted — "last used" derives from proposals.algorithm_version.
//!
//! Which version runs is a separate pointer: `app_settings`
//! `active_algorithm_version` (9 = baseline). Adopting makes the new version
//! active; "Make active" rolls back/forward without touching any row. An
//! unset or dangling pointer falls back to the newest row.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BASELINE_VERSION: i32 = 9;
/// app_settings key holding the active version number.
pub const ACTIVE_SETTING: &str = "active_algorithm_version";
const ARCHIVE_VERSIONS_BEHIND: i32 = 3;
const ARCHIVE_UNUSED_MONTHS: u32 = 3;

// ============================================================
// Rules schema (v1) — mirrors what scripts/propose.py consumes.
// deny_unknown_fields guards against prompt drift: Claude inventing rule
// keys fails validation instead of being silently stored and ignored.
// The meaning/direction of every key is documented for Claude in
// prompts/proposal-editor.md — keep the two in sync.
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
    pub variety_penalty_multiplier: HashMap<String, f64>,
    #[serde(default)]
    pub variety_penalty_per_class: Option<f64>,
    #[serde(default)]
    pub sat_time_shifts: HashMap<String, String>,
    #[serde(default)]
    pub sun_time_shifts: HashMap<String, String>,
    /// Ranking bonus per earlier assignment of the same teacher to the same
    /// weekday + start time this month (0/absent = disabled = baseline).
    #[serde(default)]
    pub slot_continuity_bonus: Option<f64>,
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
        Err(format!("{ctx}: bad time '{t}' (use 24-hour HH:MM, e.g. 08:30)"))
    }
}

/// Time-shift maps are {old start time → new start time}. Both sides must
/// be HH:MM, a shift must actually move the class, and chains (a new time
/// that is itself shifted) are rejected — propose.py applies the map in one
/// pass, so a chain would move a class twice or clobber another slot.
fn check_time_shifts(map: &HashMap<String, String>, ctx: &str) -> Result<(), String> {
    for (from, to) in map {
        check_time(from, &format!("{ctx} key (the CURRENT start time)"))?;
        check_time(to, &format!("{ctx} value for '{from}' (the NEW start time)"))?;
        if from == to {
            return Err(format!("{ctx}: '{from}' maps to itself — drop the entry"));
        }
        if map.contains_key(to) {
            return Err(format!(
                "{ctx}: '{from}' → '{to}', but '{to}' is itself shifted — chained shifts aren't supported"
            ));
        }
    }
    let mut targets: HashMap<&str, &str> = HashMap::new();
    for (from, to) in map {
        if let Some(other) = targets.insert(to.as_str(), from.as_str()) {
            return Err(format!(
                "{ctx}: both '{other}' and '{from}' move to '{to}' — two classes can't share a start time"
            ));
        }
    }
    Ok(())
}

/// Parse + validate a raw rules JSON value into the typed schema. Errors on
/// unknown keys, unknown weekday names, malformed times, and nonsensical
/// numbers. Structural only — see `validate_rules_in_context` for the checks
/// that need the database (teacher ids, class names).
pub fn validate_rules(raw: &Value) -> Result<Rules, String> {
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
    for (uid, mult) in &rules.variety_penalty_multiplier {
        uid.parse::<i32>().map_err(|_| {
            format!("variety_penalty_multiplier: key '{uid}' is not a teacher id (use the sling_user_id as a string)")
        })?;
        if !mult.is_finite() || *mult < 0.0 {
            return Err(format!(
                "variety_penalty_multiplier: teacher {uid} has {mult} — use a number ≥ 0 (1.0 = default)"
            ));
        }
    }
    if let Some(p) = rules.variety_penalty_per_class {
        if !p.is_finite() || p < 0.0 {
            return Err(format!(
                "variety_penalty_per_class: {p} — use a number ≥ 0 (baseline is 0.3)"
            ));
        }
    }
    if let Some(b) = rules.slot_continuity_bonus {
        if !b.is_finite() || b < 0.0 {
            return Err(format!(
                "slot_continuity_bonus: {b} — use a number ≥ 0 (0 = off; 1.0 is a mild, 3.0 a strong preference)"
            ));
        }
    }
    check_time_shifts(&rules.sat_time_shifts, "sat_time_shifts")?;
    check_time_shifts(&rules.sun_time_shifts, "sun_time_shifts")?;
    Ok(rules)
}

/// What rules may reference: every known teacher id and class name.
#[derive(Debug, Default, Clone)]
pub struct RuleContext {
    pub teacher_ids: HashSet<i32>,
    pub class_names: HashSet<String>,
}

pub fn load_rule_context(conn: &duckdb::Connection) -> Result<RuleContext, String> {
    let teacher_ids: HashSet<i32> = {
        let mut stmt = conn
            .prepare("SELECT sling_user_id FROM teachers")
            .map_err(err)?;
        stmt.query_map([], |r| r.get(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
    };
    let class_names: HashSet<String> = {
        let mut stmt = conn.prepare("SELECT class_name FROM positions").map_err(err)?;
        stmt.query_map([], |r| r.get(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
    };
    Ok(RuleContext { teacher_ids, class_names })
}

/// Structural validation plus referential checks: every teacher id and
/// class name a rule mentions must exist in the database. Catches the
/// "Claude guessed an id / misspelled a class" failure mode, where the rule
/// would otherwise be stored and silently match nothing.
pub fn validate_rules_in_context(raw: &Value, ctx: &RuleContext) -> Result<Rules, String> {
    let rules = validate_rules(raw)?;
    let known_classes = || {
        let mut v: Vec<&str> = ctx.class_names.iter().map(String::as_str).collect();
        v.sort();
        v.join(", ")
    };
    let teacher = |uid: i32, where_: &str| -> Result<(), String> {
        if ctx.teacher_ids.contains(&uid) {
            Ok(())
        } else {
            Err(format!("{where_}: teacher id {uid} is not in the roster"))
        }
    };
    let class = |name: &str, where_: &str| -> Result<(), String> {
        if ctx.class_names.contains(name) {
            Ok(())
        } else {
            Err(format!(
                "{where_}: '{name}' is not a known class (known: {})",
                known_classes()
            ))
        }
    };
    for b in &rules.teacher_class_blocklist {
        teacher(b.sling_user_id, "teacher_class_blocklist")?;
        class(&b.class_name, "teacher_class_blocklist")?;
    }
    for b in &rules.teacher_slot_blocklist {
        teacher(b.sling_user_id, "teacher_slot_blocklist")?;
    }
    for p in &rules.priority_slots {
        teacher(p.sling_user_id, "priority_slots")?;
    }
    for o in &rules.slot_class_overrides {
        class(&o.class_name, "slot_class_overrides")?;
    }
    for uid in rules.variety_penalty_multiplier.keys() {
        // Parse already checked in validate_rules.
        teacher(uid.parse().unwrap_or_default(), "variety_penalty_multiplier")?;
    }
    Ok(rules)
}

// ============================================================
// Rules diff (shown before Adopt)
// ============================================================

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct RuleDiffEntry {
    pub rule_key: String,
    /// What identifies the entry within its key (e.g. "501 · Reform"), or
    /// "" for scalar keys.
    pub identity: String,
    /// "added" | "removed" | "changed"
    pub kind: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

const RULE_KEY_ORDER: &[&str] = &[
    "teacher_class_blocklist",
    "teacher_slot_blocklist",
    "priority_slots",
    "slot_class_overrides",
    "variety_penalty_multiplier",
    "variety_penalty_per_class",
    "sat_time_shifts",
    "sun_time_shifts",
    "slot_continuity_bonus",
];

fn identity_fields(key: &str) -> &'static [&'static str] {
    match key {
        "teacher_class_blocklist" => &["sling_user_id", "class_name"],
        "teacher_slot_blocklist" | "priority_slots" => &["sling_user_id", "weekday", "time"],
        "slot_class_overrides" => &["weekday", "time"],
        _ => &[],
    }
}

fn value_label(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Flatten one rule key's value into identity → entry.
fn rule_entries(key: &str, v: Option<&Value>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    match v {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            let fields = identity_fields(key);
            for item in items {
                let id = if fields.is_empty() {
                    item.to_string()
                } else {
                    fields
                        .iter()
                        .map(|f| item.get(*f).map(value_label).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(" · ")
                };
                out.insert(id, item.clone());
            }
        }
        Some(Value::Object(map)) => {
            for (k, val) in map {
                out.insert(k.clone(), val.clone());
            }
        }
        Some(scalar) => {
            out.insert(String::new(), scalar.clone());
        }
    }
    out
}

/// Entry-level diff of two rule snapshots. "removed" entries are the ones
/// to worry about: Claude is asked for the FULL rule set, and a dropped
/// entry there silently deletes a standing rule.
pub fn diff_rules(active: &Value, candidate: &Value) -> Vec<RuleDiffEntry> {
    let mut keys: Vec<String> = RULE_KEY_ORDER.iter().map(|s| s.to_string()).collect();
    let mut extra: Vec<String> = [active, candidate]
        .iter()
        .filter_map(|v| v.as_object())
        .flat_map(|m| m.keys().cloned())
        .filter(|k| !RULE_KEY_ORDER.contains(&k.as_str()))
        .collect();
    extra.sort();
    extra.dedup();
    keys.extend(extra);

    let mut out = Vec::new();
    for key in keys {
        let before = rule_entries(&key, active.get(&key));
        let after = rule_entries(&key, candidate.get(&key));
        let ids: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
        for id in ids {
            let (b, a) = (before.get(id), after.get(id));
            let kind = match (b, a) {
                (Some(b), Some(a)) if b == a => continue,
                (Some(_), Some(_)) => "changed",
                (Some(_), None) => "removed",
                (None, Some(_)) => "added",
                (None, None) => continue,
            };
            out.push(RuleDiffEntry {
                rule_key: key.clone(),
                identity: id.clone(),
                kind: kind.to_string(),
                before: b.cloned(),
                after: a.cloned(),
            });
        }
    }
    out
}

// ============================================================
// Version store
// ============================================================

#[derive(Debug, Serialize, Clone)]
pub struct AlgorithmVersion {
    pub version: i32,
    pub description: String,
    pub rules: Value,
    pub script_file: Option<String>,
    pub created_by: String,
    pub adopted_at: String,
    pub last_used_month: Option<String>,
    pub script_archived: bool,
    pub script_missing: bool,
    /// sha256 of the shipped propose.py when this version was adopted.
    pub baseline_sha256: Option<String>,
    /// True when this version runs a custom script built on a different
    /// shipped baseline than the one installed now.
    pub baseline_outdated: bool,
    pub is_active: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// sha256 of the shipped baseline script, if readable.
pub fn shipped_script_sha(project_root: &Path) -> Option<String> {
    std::fs::read(project_root.join("scripts").join("propose.py"))
        .ok()
        .map(|b| sha256_hex(&b))
}

const VERSION_COLUMNS: &str = concat!("version, description, CAST(rules AS VARCHAR), script_file, created_by,
     ", crate::db::utc_iso!("adopted_at"), ", baseline_sha256");

type VersionRow = (i32, String, String, Option<String>, String, String, Option<String>);

fn read_row(r: &duckdb::Row<'_>) -> duckdb::Result<VersionRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
    ))
}

fn row_to_version(
    conn: &duckdb::Connection,
    row: VersionRow,
    algo_dir: Option<&Path>,
    shipped_sha: Option<&str>,
) -> AlgorithmVersion {
    let (version, description, rules_text, script_file, created_by, adopted_at, baseline_sha256) =
        row;
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
    let baseline_outdated = match (&script_file, &baseline_sha256, shipped_sha) {
        (Some(_), Some(base), Some(now)) => base != now,
        _ => false,
    };
    AlgorithmVersion {
        version,
        description,
        rules: serde_json::from_str(&rules_text).unwrap_or(Value::Null),
        script_file,
        created_by,
        adopted_at,
        last_used_month,
        script_archived,
        script_missing,
        baseline_sha256,
        baseline_outdated,
        is_active: false,
    }
}

/// One adopted version by number (None if no such row).
pub fn get_version(
    conn: &duckdb::Connection,
    version: i32,
) -> Result<Option<AlgorithmVersion>, String> {
    let row = conn.query_row(
        &format!("SELECT {VERSION_COLUMNS} FROM algorithm_versions WHERE version = ?"),
        duckdb::params![version],
        read_row,
    );
    match row {
        Ok(r) => Ok(Some(row_to_version(conn, r, None, None))),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(err(e)),
    }
}

fn newest_version(conn: &duckdb::Connection) -> Result<Option<AlgorithmVersion>, String> {
    let row = conn.query_row(
        &format!("SELECT {VERSION_COLUMNS} FROM algorithm_versions ORDER BY version DESC LIMIT 1"),
        [],
        read_row,
    );
    match row {
        Ok(r) => Ok(Some(row_to_version(conn, r, None, None))),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(err(e)),
    }
}

fn active_setting(conn: &duckdb::Connection) -> Option<i32> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?",
        duckdb::params![ACTIVE_SETTING],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|s| s.trim().parse().ok())
}

/// The version generate_proposal runs, or None for the v9 baseline. Reads
/// the app_settings pointer; unset or dangling → the newest adopted row.
pub fn active_version(conn: &duckdb::Connection) -> Result<Option<AlgorithmVersion>, String> {
    match active_setting(conn) {
        Some(BASELINE_VERSION) => return Ok(None),
        Some(v) => {
            if let Some(found) = get_version(conn, v)? {
                return Ok(Some(found));
            }
        }
        None => {}
    }
    newest_version(conn)
}

/// Active version number (9 = baseline).
pub fn active_version_number(conn: &duckdb::Connection) -> Result<i32, String> {
    Ok(active_version(conn)?
        .map(|v| v.version)
        .unwrap_or(BASELINE_VERSION))
}

fn write_active_setting(conn: &duckdb::Connection, version: i32) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?, ?, now())",
        duckdb::params![ACTIVE_SETTING, version.to_string()],
    )
    .map_err(err)?;
    Ok(())
}

/// Point generate_proposal at `version` (9 = shipped baseline). Refuses a
/// version whose script file has been deleted — it could not run.
pub fn set_active_version(
    conn: &duckdb::Connection,
    algo_dir: &Path,
    version: i32,
) -> Result<(), String> {
    if version != BASELINE_VERSION {
        let v = get_version(conn, version)?
            .ok_or_else(|| format!("version v{version} does not exist"))?;
        if let Some(f) = &v.script_file {
            if !algo_dir.join(f).exists() && !algo_dir.join("archive").join(f).exists() {
                return Err(format!(
                    "v{version}'s script {f} was deleted — it can't be made active"
                ));
            }
        }
    }
    write_active_setting(conn, version)
}

/// All adopted versions, newest first, with script file status and the
/// active flag. `shipped_sha` (sha256 of the installed propose.py) drives
/// `baseline_outdated`.
pub fn list_versions(
    conn: &duckdb::Connection,
    algo_dir: &Path,
    shipped_sha: Option<&str>,
) -> Result<Vec<AlgorithmVersion>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {VERSION_COLUMNS} FROM algorithm_versions ORDER BY version DESC"
        ))
        .map_err(err)?;
    let rows: Vec<VersionRow> = stmt
        .query_map([], read_row)
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let active = active_version_number(conn)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let mut v = row_to_version(conn, r, Some(algo_dir), shipped_sha);
            v.is_active = v.version == active;
            v
        })
        .collect())
}

/// Adopt a new version: validate the rules, assign version = max + 1
/// (starting at 10), write the script file (if any) BEFORE inserting the
/// row, insert append-only, and make it the active version. Returns the new
/// version number.
///
/// With no `script_content` (a rules-only adoption) the new version reuses
/// the ACTIVE version's script file — never the shipped baseline by default,
/// or adopting a rule tweak would silently discard an adopted code change.
/// `shipped_sha` is the sha256 of the installed baseline propose.py; it is
/// recorded for new scripts (rules-only versions inherit the reused
/// script's value).
pub fn adopt_version(
    conn: &duckdb::Connection,
    algo_dir: &Path,
    description: &str,
    rules_raw: &Value,
    script_content: Option<&str>,
    claude_run_id: Option<i64>,
    shipped_sha: Option<&str>,
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

    let (script_file, baseline_sha) = match script_content {
        Some(content) => {
            let name = format!("propose_v{version}.py");
            std::fs::create_dir_all(algo_dir).map_err(err)?;
            std::fs::write(algo_dir.join(&name), content).map_err(err)?;
            (Some(name), shipped_sha.map(str::to_string))
        }
        None => match active_version(conn)? {
            Some(active) => (active.script_file, active.baseline_sha256),
            None => (None, None),
        },
    };

    let created_by = if claude_run_id.is_some() { "claude" } else { "user" };
    conn.execute(
        "INSERT INTO algorithm_versions
            (version, description, rules, script_file, created_by, claude_run_id, baseline_sha256)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        duckdb::params![
            version,
            description,
            serde_json::to_string(rules_raw).map_err(err)?,
            script_file,
            created_by,
            claude_run_id,
            baseline_sha
        ],
    )
    .map_err(err)?;
    write_active_setting(conn, version)?;
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
                "algorithm script {f} was deleted — make another version active or re-adopt the rules on the baseline"
            ))
        }
    }
}

/// Script path for the active version (or the shipped baseline).
pub fn resolve_active_script(
    algo_dir: &Path,
    active: Option<&AlgorithmVersion>,
    project_root: &Path,
) -> Result<PathBuf, String> {
    match active {
        Some(v) => resolve_script(algo_dir, v, project_root),
        None => Ok(project_root.join("scripts").join("propose.py")),
    }
}

/// Startup sweep: move script files that are more than
/// ARCHIVE_VERSIONS_BEHIND versions behind the active one AND unused for
/// ARCHIVE_UNUSED_MONTHS months (or never used) into algorithms/archive/.
/// The active version's script is never moved (rules-only versions share
/// their predecessor's file). Returns the moved file names. Deletion stays
/// manual-only.
pub fn archive_sweep(conn: &duckdb::Connection, algo_dir: &Path) -> Result<Vec<String>, String> {
    archive_sweep_at(conn, algo_dir, chrono::Utc::now())
}

/// "YYYY-MM" ARCHIVE_UNUSED_MONTHS before the studio's month at `now`.
fn archive_cutoff_month(now: chrono::DateTime<chrono::Utc>) -> Result<String, String> {
    let current = crate::sling::studio_month_at(now);
    let (y, m): (i32, i32) = {
        let (y, m) = current.split_once('-').ok_or("bad month")?;
        (y.parse().map_err(err)?, m.parse().map_err(err)?)
    };
    let mut y2 = y;
    let mut m2 = m - ARCHIVE_UNUSED_MONTHS as i32;
    while m2 < 1 {
        m2 += 12;
        y2 -= 1;
    }
    Ok(format!("{y2:04}-{m2:02}"))
}

/// `archive_sweep` with an injected clock (tests).
fn archive_sweep_at(
    conn: &duckdb::Connection,
    algo_dir: &Path,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<String>, String> {
    let versions = list_versions(conn, algo_dir, None)?;
    if versions.is_empty() {
        return Ok(Vec::new());
    }
    let active = active_version(conn)?;
    let active_num = active.as_ref().map(|v| v.version).unwrap_or(BASELINE_VERSION);
    let active_file = active.and_then(|v| v.script_file);
    // Studio month (US Central), not the DB's UTC now().
    let cutoff_month = archive_cutoff_month(now)?;

    let mut moved = Vec::new();
    for v in &versions {
        let Some(file) = &v.script_file else { continue };
        if active_file.as_deref() == Some(file.as_str()) {
            continue;
        }
        if v.version >= active_num - ARCHIVE_VERSIONS_BEHIND {
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
        let c = crate::db::open_in_memory().expect("open");
        crate::migrations::run(&c).expect("migrations");
        c
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bk-algo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn adopt(c: &duckdb::Connection, dir: &Path, rules: Value, script: Option<&str>) -> i32 {
        adopt_version(c, dir, "v", &rules, script, None, Some("sha-now")).unwrap()
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
        assert!(validate_rules(&json!({"variety_penalty_multiplier": {"501": -1.0}})).is_err());
        assert!(validate_rules(&json!({"variety_penalty_per_class": -0.1})).is_err());
        assert!(validate_rules(&json!({"slot_continuity_bonus": -1.0})).is_err());
        assert_eq!(
            validate_rules(&json!({"slot_continuity_bonus": 2.5})).unwrap().slot_continuity_bonus,
            Some(2.5)
        );
        assert!(validate_rules(&json!({"slot_continuity_bonus": 0})).is_ok());
        let ok = validate_rules(&json!({
            "teacher_class_blocklist": [{"sling_user_id": 501, "class_name": "Reform", "reason": "r"}],
            "priority_slots": [{"sling_user_id": 501, "weekday": "Mon", "time": "09:00"}],
            "variety_penalty_per_class": 0.5
        }))
        .unwrap();
        assert_eq!(ok.teacher_class_blocklist.len(), 1);
    }

    #[test]
    fn validate_time_shifts() {
        assert!(validate_rules(&json!({"sat_time_shifts": {"08:00": "08:30"}})).is_ok());
        // Malformed key or value.
        let e = validate_rules(&json!({"sat_time_shifts": {"8:00": "08:30"}})).unwrap_err();
        assert!(e.contains("CURRENT start time"), "{e}");
        let e = validate_rules(&json!({"sun_time_shifts": {"08:00": "8.30am"}})).unwrap_err();
        assert!(e.contains("NEW start time"), "{e}");
        // No-op, chain, collision.
        assert!(validate_rules(&json!({"sat_time_shifts": {"08:00": "08:00"}})).is_err());
        let e = validate_rules(&json!({"sat_time_shifts": {"08:00": "09:00", "09:00": "10:00"}}))
            .unwrap_err();
        assert!(e.contains("chained"), "{e}");
        let e = validate_rules(&json!({"sat_time_shifts": {"08:00": "10:00", "09:00": "10:00"}}))
            .unwrap_err();
        assert!(e.contains("share a start time"), "{e}");
    }

    #[test]
    fn validate_rules_in_context_checks_references() {
        let ctx = RuleContext {
            teacher_ids: [501, 502].into_iter().collect(),
            class_names: ["Classic".to_string(), "Reform".to_string()].into_iter().collect(),
        };
        assert!(validate_rules_in_context(
            &json!({
                "teacher_class_blocklist": [{"sling_user_id": 501, "class_name": "Reform"}],
                "slot_class_overrides": [{"weekday": "Tue", "time": "17:30", "class_name": "Classic"}],
                "variety_penalty_multiplier": {"502": 2.0}
            }),
            &ctx
        )
        .is_ok());
        let e = validate_rules_in_context(
            &json!({"teacher_class_blocklist": [{"sling_user_id": 999, "class_name": "Reform"}]}),
            &ctx,
        )
        .unwrap_err();
        assert!(e.contains("999") && e.contains("roster"), "{e}");
        let e = validate_rules_in_context(
            &json!({"slot_class_overrides": [{"weekday": "Tue", "time": "17:30", "class_name": "Reformer"}]}),
            &ctx,
        )
        .unwrap_err();
        assert!(e.contains("'Reformer'") && e.contains("Classic, Reform"), "{e}");
        assert!(validate_rules_in_context(
            &json!({"priority_slots": [{"sling_user_id": 7, "weekday": "Mon", "time": "09:00"}]}),
            &ctx
        )
        .is_err());
        assert!(validate_rules_in_context(&json!({"variety_penalty_multiplier": {"7": 1.5}}), &ctx)
            .is_err());
    }

    #[test]
    fn diff_rules_reports_added_removed_changed() {
        let active = json!({
            "teacher_class_blocklist": [
                {"sling_user_id": 501, "class_name": "Reform", "reason": "a"},
                {"sling_user_id": 502, "class_name": "Classic", "reason": "b"}
            ],
            "variety_penalty_per_class": 0.3,
            "sat_time_shifts": {"08:00": "08:30"}
        });
        let candidate = json!({
            "teacher_class_blocklist": [
                {"sling_user_id": 501, "class_name": "Reform", "reason": "a"}
            ],
            "variety_penalty_per_class": 0.5,
            "sat_time_shifts": {"08:00": "08:30"},
            "priority_slots": [{"sling_user_id": 503, "weekday": "Mon", "time": "09:00"}]
        });
        let d = diff_rules(&active, &candidate);
        let summary: Vec<(String, String, String)> = d
            .iter()
            .map(|e| (e.rule_key.clone(), e.identity.clone(), e.kind.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("teacher_class_blocklist".into(), "502 · Classic".into(), "removed".into()),
                ("priority_slots".into(), "503 · Mon · 09:00".into(), "added".into()),
                ("variety_penalty_per_class".into(), "".into(), "changed".into()),
            ]
        );
        assert!(diff_rules(&active, &active).is_empty());
    }

    #[test]
    fn adopt_assigns_sequential_versions_and_writes_scripts() {
        let c = conn();
        let dir = scratch("adopt");
        let v1 = adopt_version(&c, &dir, "v10 — rules only", &json!({}), None, None, None).unwrap();
        assert_eq!(v1, 10);
        let v2 = adopt_version(
            &c,
            &dir,
            "v11 — code",
            &json!({}),
            Some("print('hi')"),
            Some(42),
            Some("abc"),
        )
        .unwrap();
        assert_eq!(v2, 11);
        assert!(dir.join("propose_v11.py").exists());

        let versions = list_versions(&c, &dir, Some("abc")).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 11);
        assert_eq!(versions[0].created_by, "claude");
        assert_eq!(versions[0].baseline_sha256.as_deref(), Some("abc"));
        assert!(!versions[0].baseline_outdated);
        assert!(versions[0].is_active && !versions[1].is_active);
        assert_eq!(versions[1].created_by, "user");
        assert!(active_version(&c).unwrap().unwrap().version == 11);

        // A newer shipped baseline flags the custom script as outdated.
        let versions = list_versions(&c, &dir, Some("def")).unwrap();
        assert!(versions[0].baseline_outdated);
        assert!(!versions[1].baseline_outdated, "baseline-script versions never outdated");

        // Invalid rules refuse adoption and burn no version number.
        assert!(adopt_version(&c, &dir, "bad", &json!({"nope": 1}), None, None, None).is_err());
        assert_eq!(active_version(&c).unwrap().unwrap().version, 11);
    }

    /// Regression: a rules-only adoption after a code adoption used to write
    /// script_file = NULL and silently revert to the shipped propose.py.
    #[test]
    fn rules_only_adoption_carries_the_active_script_forward() {
        let c = conn();
        let dir = scratch("carry");
        let v10 = adopt(&c, &dir, json!({}), Some("custom code"));
        let v11 = adopt(
            &c,
            &dir,
            json!({"variety_penalty_per_class": 0.4}),
            None,
        );
        assert_eq!((v10, v11), (10, 11));
        let ten = get_version(&c, 10).unwrap().unwrap();
        let eleven = get_version(&c, 11).unwrap().unwrap();
        assert_eq!(ten.script_file.as_deref(), Some("propose_v10.py"));
        assert_eq!(eleven.script_file, ten.script_file);
        assert_eq!(eleven.baseline_sha256, ten.baseline_sha256);

        // Rolling back to the baseline and adopting rules-only runs the
        // shipped script again (the carried script is the ACTIVE one).
        set_active_version(&c, &dir, BASELINE_VERSION).unwrap();
        let v12 = adopt(&c, &dir, json!({}), None);
        assert!(get_version(&c, v12).unwrap().unwrap().script_file.is_none());
    }

    #[test]
    fn active_pointer_rollback_and_fallbacks() {
        let c = conn();
        let dir = scratch("active");
        // Nothing adopted: baseline.
        assert!(active_version(&c).unwrap().is_none());
        assert_eq!(active_version_number(&c).unwrap(), 9);

        adopt(&c, &dir, json!({}), None);
        adopt(&c, &dir, json!({}), Some("code"));
        assert_eq!(active_version_number(&c).unwrap(), 11);

        // Roll back to v10, then to the baseline, then forward again.
        set_active_version(&c, &dir, 10).unwrap();
        assert_eq!(active_version_number(&c).unwrap(), 10);
        let listed = list_versions(&c, &dir, None).unwrap();
        assert!(listed.iter().find(|v| v.version == 10).unwrap().is_active);
        set_active_version(&c, &dir, 9).unwrap();
        assert!(active_version(&c).unwrap().is_none());
        set_active_version(&c, &dir, 11).unwrap();
        assert_eq!(active_version_number(&c).unwrap(), 11);

        // Unknown version refused; deleted script refused.
        assert!(set_active_version(&c, &dir, 42).is_err());
        std::fs::remove_file(dir.join("propose_v11.py")).unwrap();
        set_active_version(&c, &dir, 10).unwrap();
        let e = set_active_version(&c, &dir, 11).unwrap_err();
        assert!(e.contains("deleted"), "{e}");

        // Dangling or garbage pointer falls back to the newest row.
        c.execute(
            "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, '77')",
            duckdb::params![ACTIVE_SETTING],
        )
        .unwrap();
        assert_eq!(active_version_number(&c).unwrap(), 11);
        c.execute(
            "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?, 'junk')",
            duckdb::params![ACTIVE_SETTING],
        )
        .unwrap();
        assert_eq!(active_version_number(&c).unwrap(), 11);

        // Adopting makes the new version active even after a rollback.
        set_active_version(&c, &dir, 10).unwrap();
        let v12 = adopt(&c, &dir, json!({}), None);
        assert_eq!(active_version_number(&c).unwrap(), v12);
    }

    #[test]
    fn resolve_script_falls_back_to_archive_then_errors() {
        let c = conn();
        let dir = scratch("resolve");
        let root = scratch("resolve-root");
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::write(root.join("scripts/propose.py"), "baseline").unwrap();

        // Baseline (no rows): rules-only adopt resolves to the shipped script.
        let v = adopt(&c, &dir, json!({}), None);
        let v10 = get_version(&c, v).unwrap().unwrap();
        assert_eq!(
            resolve_script(&dir, &v10, &root).unwrap(),
            root.join("scripts/propose.py")
        );
        assert_eq!(
            shipped_script_sha(&root).unwrap(),
            sha256_hex(b"baseline")
        );

        // Script version: live → archive → error.
        adopt(&c, &dir, json!({}), Some("code"));
        let v11 = get_version(&c, 11).unwrap().unwrap();
        assert_eq!(resolve_script(&dir, &v11, &root).unwrap(), dir.join("propose_v11.py"));

        std::fs::create_dir_all(dir.join("archive")).unwrap();
        std::fs::rename(dir.join("propose_v11.py"), dir.join("archive/propose_v11.py")).unwrap();
        assert_eq!(
            resolve_script(&dir, &v11, &root).unwrap(),
            dir.join("archive/propose_v11.py")
        );

        std::fs::remove_file(dir.join("archive/propose_v11.py")).unwrap();
        let e = resolve_script(&dir, &v11, &root).unwrap_err();
        assert!(e.contains("make another version active"), "{e}");
    }

    #[test]
    fn archive_sweep_only_old_and_unused() {
        let c = conn();
        let dir = scratch("sweep");
        // v10..v15, all with script files; active = 15, so v10 and v11 are
        // more than 3 versions behind.
        for _ in 10..=15 {
            adopt(&c, &dir, json!({}), Some("code"));
        }
        // v11 used recently (protected); v10 used long ago (sweepable);
        // v12..15 are within 3 of active regardless of use.
        c.execute_batch(
            "INSERT INTO proposals (target_month, algorithm_version, parameters, generated_at)
             VALUES ('2026-09', 'v11', '{}', TIMESTAMPTZ '2026-09-01 12:00:00+00');
             INSERT INTO proposals (target_month, algorithm_version, parameters, generated_at)
             VALUES ('2026-02', 'v10', '{}', TIMESTAMPTZ '2026-02-01 12:00:00+00');",
        )
        .unwrap();
        let now = utc("2026-09-15T12:00:00Z");

        let moved = archive_sweep_at(&c, &dir, now).unwrap();
        assert_eq!(moved, vec!["propose_v10.py".to_string()]);
        assert!(dir.join("archive/propose_v10.py").exists());
        assert!(dir.join("propose_v11.py").exists(), "recently used stays");
        assert!(dir.join("propose_v12.py").exists(), "within 3 of active stays");

        // Idempotent: second sweep moves nothing.
        assert!(archive_sweep_at(&c, &dir, now).unwrap().is_empty());
    }

    fn utc(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&chrono::Utc)
    }

    #[test]
    fn archive_cutoff_uses_the_studio_month_not_utc() {
        // 2026-10-01 00:30Z is still Sept 30, 7:30pm CDT at the studio.
        assert_eq!(archive_cutoff_month(utc("2026-10-01T00:30:00Z")).unwrap(), "2026-06");
        assert_eq!(archive_cutoff_month(utc("2026-10-01T05:00:00Z")).unwrap(), "2026-07");
        // January wraps into the previous year.
        assert_eq!(archive_cutoff_month(utc("2027-01-15T12:00:00Z")).unwrap(), "2026-10");
    }

    #[test]
    fn archive_sweep_never_moves_the_active_script() {
        let c = conn();
        let dir = scratch("sweep-active");
        adopt(&c, &dir, json!({}), Some("code")); // v10 owns the script
        for _ in 11..=15 {
            adopt(&c, &dir, json!({}), None); // rules-only: all reuse propose_v10.py
        }
        assert_eq!(
            get_version(&c, 15).unwrap().unwrap().script_file.as_deref(),
            Some("propose_v10.py")
        );
        assert!(archive_sweep(&c, &dir).unwrap().is_empty());
        assert!(dir.join("propose_v10.py").exists());
    }
}
