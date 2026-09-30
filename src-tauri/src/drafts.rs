// Multiple drafts per month (migration 0012).
//
// A draft is a `proposals` row. Its name / parent / archived flag live in
// `proposal_drafts`; which draft Push sends lives in `month_push_candidate`.
// `proposals` itself is never UPDATEd here (incoming FKs — see CLAUDE.md).
//
// The push draft is the single source of truth for "the" draft of a month:
// Sling's dedupe matches (date, time, teacher, position), so pushing a second
// draft would ADD its differing shifts on top of the first. Push therefore
// refuses anything but the push draft (`ensure_push_candidate`).

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use tauri::State;

use crate::commands::{compare_runs, weekday_of, RunShift};
use crate::db::Db;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

pub const MAX_NAME_LEN: usize = 60;

/// Trimmed, non-empty, bounded draft name.
pub fn clean_name(name: &str) -> Result<String, String> {
    let t = name.trim();
    if t.is_empty() {
        return Err("draft name can't be empty".to_string());
    }
    if t.chars().count() > MAX_NAME_LEN {
        return Err(format!("draft name is too long (max {MAX_NAME_LEN} characters)"));
    }
    Ok(t.to_string())
}

fn month_of(conn: &duckdb::Connection, proposal_id: i64) -> Result<String, String> {
    conn.query_row(
        "SELECT target_month FROM proposals WHERE id = ?",
        duckdb::params![proposal_id],
        |r| r.get(0),
    )
    .map_err(|e| format!("draft {proposal_id} not found: {e:#}"))
}

/// Display name of a draft (falls back to "Draft #id" for a proposal with no
/// metadata row, which only a pre-0012 writer could produce).
pub fn draft_name(conn: &duckdb::Connection, proposal_id: i64) -> String {
    conn.query_row(
        "SELECT name FROM proposal_drafts WHERE proposal_id = ?",
        duckdb::params![proposal_id],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| format!("Draft #{proposal_id}"))
}

/// "Draft N" where N is the month's draft count including the new one,
/// bumped until it doesn't collide with an existing name in the month.
fn next_draft_name(conn: &duckdb::Connection, month: &str) -> Result<String, String> {
    let names: Vec<String> = conn
        .prepare(
            "SELECT d.name FROM proposal_drafts d
             JOIN proposals p ON p.id = d.proposal_id
             WHERE p.target_month = ?",
        )
        .map_err(err)?
        .query_map(duckdb::params![month], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let mut n = names.len() + 1;
    loop {
        let candidate = format!("Draft {n}");
        if !names.iter().any(|x| x == &candidate) {
            return Ok(candidate);
        }
        n += 1;
    }
}

/// Record a freshly generated proposal as a draft. The push draft is set
/// only when the month has none — generating a what-if never silently
/// changes what Push sends.
pub fn record_generated(
    conn: &duckdb::Connection,
    proposal_id: i64,
    month: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let name = match name {
        Some(n) => clean_name(n)?,
        None => next_draft_name(conn, month)?,
    };
    conn.execute(
        "INSERT INTO proposal_drafts (proposal_id, name, created_from) VALUES (?, ?, 'generate')",
        duckdb::params![proposal_id, name],
    )
    .map_err(err)?;
    conn.execute(
        "INSERT INTO month_push_candidate (target_month, proposal_id)
         SELECT ?, ? WHERE NOT EXISTS
             (SELECT 1 FROM month_push_candidate WHERE target_month = ?)",
        duckdb::params![month, proposal_id, month],
    )
    .map_err(err)?;
    Ok(())
}

pub fn push_candidate(conn: &duckdb::Connection, month: &str) -> Result<Option<i64>, String> {
    match conn.query_row(
        "SELECT proposal_id FROM month_push_candidate WHERE target_month = ?",
        duckdb::params![month],
        |r| r.get::<_, i64>(0),
    ) {
        Ok(id) => Ok(Some(id)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(err(e)),
    }
}

/// Push guard: only the month's push draft may be pushed.
pub fn ensure_push_candidate(conn: &duckdb::Connection, proposal_id: i64) -> Result<(), String> {
    let month = month_of(conn, proposal_id)?;
    match push_candidate(conn, &month)? {
        Some(id) if id == proposal_id => Ok(()),
        Some(id) => Err(format!(
            "\"{}\" is not the push draft for {month} — \"{}\" is. Mark this draft as the \
             push draft (\"Use for push\") first. Only one draft per month is pushed: pushing \
             a second one would add its differing shifts to Sling on top of the first.",
            draft_name(conn, proposal_id),
            draft_name(conn, id),
        )),
        None => Err(format!(
            "{month} has no push draft yet. Mark \"{}\" as the push draft (\"Use for push\") first.",
            draft_name(conn, proposal_id)
        )),
    }
}

/// Copy a draft: the proposals row (same generated_at, so staleness vs the
/// last pull carries over; is_current FALSE) and every shift with fresh ids,
/// remapping coteach_partner_shift_id onto the copies. Edit and push history
/// are not copied. One transaction; no UPDATEs (ids are pre-allocated).
pub fn duplicate_impl(
    conn: &mut duckdb::Connection,
    proposal_id: i64,
    name: Option<&str>,
) -> Result<i64, String> {
    let tx = conn.transaction().map_err(err)?;
    let parent_name = draft_name(&tx, proposal_id);
    month_of(&tx, proposal_id)?;
    let name = match name {
        Some(n) => clean_name(n)?,
        None => {
            let base = format!("Copy of {parent_name}");
            base.chars().take(MAX_NAME_LEN).collect()
        }
    };

    let new_id: i64 = tx
        .query_row(
            "INSERT INTO proposals (target_month, algorithm_version, parameters, generated_at, notes, is_current)
             SELECT target_month, algorithm_version, parameters, generated_at, notes, FALSE
             FROM proposals WHERE id = ?
             RETURNING id",
            duckdb::params![proposal_id],
            |r| r.get(0),
        )
        .map_err(err)?;

    let old_rows: Vec<(i64, Option<i64>)> = tx
        .prepare("SELECT id, coteach_partner_shift_id FROM proposal_shifts WHERE proposal_id = ? ORDER BY id")
        .map_err(err)?
        .query_map(duckdb::params![proposal_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;

    let mut remap: HashMap<i64, i64> = HashMap::with_capacity(old_rows.len());
    for (old, _) in &old_rows {
        let fresh: i64 = tx
            .query_row("SELECT nextval('seq_proposal_shifts')", [], |r| r.get(0))
            .map_err(err)?;
        remap.insert(*old, fresh);
    }
    for (old, partner) in &old_rows {
        // A partner outside this proposal (never produced today) can't be
        // remapped; dropping the link beats pointing into another draft.
        let new_partner = partner.and_then(|p| remap.get(&p).copied());
        tx.execute(
            "INSERT INTO proposal_shifts (
                id, proposal_id, shift_date, start_time, end_time, sling_position_id,
                sling_user_id, generation_reason, flag, is_coteach, coteach_partner_shift_id,
                is_dropped, coteach_label)
             SELECT ?, ?, shift_date, start_time, end_time, sling_position_id,
                    sling_user_id, generation_reason, flag, is_coteach, ?,
                    is_dropped, coteach_label
             FROM proposal_shifts WHERE id = ?",
            duckdb::params![remap[old], new_id, new_partner, old],
        )
        .map_err(err)?;
    }

    tx.execute(
        "INSERT INTO proposal_drafts (proposal_id, name, parent_proposal_id, created_from)
         VALUES (?, ?, ?, 'duplicate')",
        duckdb::params![new_id, name, proposal_id],
    )
    .map_err(err)?;
    // Same shifts, so the parent's last conflict check holds for the copy
    // too (otherwise a copy of a re-checked draft would look stale).
    tx.execute(
        "INSERT INTO draft_checks (proposal_id, checked_at)
         SELECT ?, checked_at FROM draft_checks WHERE proposal_id = ?",
        duckdb::params![new_id, proposal_id],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(new_id)
}

/// Ensure a metadata row exists (pre-0012 writers never made one), so the
/// UPDATEs below have something to hit.
fn ensure_draft_row(conn: &duckdb::Connection, proposal_id: i64) -> Result<(), String> {
    month_of(conn, proposal_id)?;
    conn.execute(
        "INSERT INTO proposal_drafts (proposal_id, name)
         SELECT ?, ? WHERE NOT EXISTS (SELECT 1 FROM proposal_drafts WHERE proposal_id = ?)",
        duckdb::params![proposal_id, format!("Draft #{proposal_id}"), proposal_id],
    )
    .map_err(err)?;
    Ok(())
}

pub fn rename_impl(conn: &duckdb::Connection, proposal_id: i64, name: &str) -> Result<(), String> {
    let name = clean_name(name)?;
    ensure_draft_row(conn, proposal_id)?;
    // Compare-before-write: an unchanged name touches no row.
    conn.execute(
        "UPDATE proposal_drafts SET name = ? WHERE proposal_id = ? AND name IS DISTINCT FROM ?",
        duckdb::params![name, proposal_id, name],
    )
    .map_err(err)?;
    Ok(())
}

pub fn set_archived_impl(conn: &duckdb::Connection, proposal_id: i64, archived: bool) -> Result<(), String> {
    ensure_draft_row(conn, proposal_id)?;
    if archived {
        let month = month_of(conn, proposal_id)?;
        if push_candidate(conn, &month)? == Some(proposal_id) {
            return Err(
                "This is the month's push draft — mark another draft as the push draft before archiving it."
                    .to_string(),
            );
        }
    }
    conn.execute(
        "UPDATE proposal_drafts SET archived = ? WHERE proposal_id = ? AND archived IS DISTINCT FROM ?",
        duckdb::params![archived, proposal_id, archived],
    )
    .map_err(err)?;
    Ok(())
}

pub fn set_push_candidate_impl(conn: &duckdb::Connection, month: &str, proposal_id: i64) -> Result<(), String> {
    let actual = month_of(conn, proposal_id)?;
    if actual != month {
        return Err(format!("draft {proposal_id} belongs to {actual}, not {month}"));
    }
    let archived: bool = conn
        .query_row(
            "SELECT COALESCE((SELECT archived FROM proposal_drafts WHERE proposal_id = ?), FALSE)",
            duckdb::params![proposal_id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if archived {
        return Err("An archived draft can't be the push draft — unarchive it first.".to_string());
    }
    if push_candidate(conn, month)? == Some(proposal_id) {
        return Ok(());
    }
    conn.execute(
        "INSERT OR REPLACE INTO month_push_candidate (target_month, proposal_id, set_at) VALUES (?, ?, now())",
        duckdb::params![month, proposal_id],
    )
    .map_err(err)?;
    Ok(())
}

// ============================================================
// Compare two drafts
// ============================================================

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct DraftSlotDiff {
    pub date: String,
    pub weekday: String,
    pub start: String,
    pub class_a: Option<String>,
    pub class_b: Option<String>,
    pub teacher_a: Option<String>,
    pub teacher_b: Option<String>,
    /// "teacher" | "format" | "format_teacher" | "only_a" | "only_b"
    pub kind: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TeacherStats {
    pub classes: i64,
    /// Distinct weekday+start-time slots taught this month (lower = more
    /// consistent: 4 Tue-8:45 classes = 1 slot).
    pub distinct_slots: i64,
    /// Most common slot, e.g. "Tue 08:45".
    pub top_slot: String,
    pub top_slot_count: i64,
    /// top_slot_count / classes.
    pub top_slot_share: f64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TeacherConsistency {
    pub sling_user_id: i32,
    pub name: String,
    pub a: Option<TeacherStats>,
    pub b: Option<TeacherStats>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct DraftTotals {
    pub classes: i64,
    /// Sum of every teacher's distinct slots.
    pub distinct_slots: i64,
    /// classes / distinct_slots — average classes per teacher-slot (higher =
    /// more consistent; 4.0 means everyone repeats one weekly slot all month).
    pub classes_per_slot: f64,
}

#[derive(Serialize, Debug, Clone)]
pub struct ProposalDiff {
    pub target_month: String,
    pub a_id: i64,
    pub b_id: i64,
    pub a_name: String,
    pub b_name: String,
    pub changes: Vec<DraftSlotDiff>,
    pub teachers: Vec<TeacherConsistency>,
    pub totals_a: DraftTotals,
    pub totals_b: DraftTotals,
}

const WEEKDAY_ORDER: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn slot_sort_key(label: &str) -> (usize, String) {
    let (wd, time) = label.split_once(' ').unwrap_or((label, ""));
    (
        WEEKDAY_ORDER.iter().position(|d| *d == wd).unwrap_or(7),
        time.to_string(),
    )
}

/// Per-teacher consistency for one draft. Counts assigned, non-dropped rows
/// by `sling_user_id` (a co-teach row counts for its primary teacher).
pub fn consistency_stats(shifts: &[RunShift]) -> BTreeMap<i32, TeacherStats> {
    let mut per: BTreeMap<i32, BTreeMap<String, i64>> = BTreeMap::new();
    for s in shifts {
        let Some(uid) = s.user_id else { continue };
        if s.dropped {
            continue;
        }
        let slot = format!("{} {}", weekday_of(&s.date), s.start);
        *per.entry(uid).or_default().entry(slot).or_default() += 1;
    }
    per.into_iter()
        .map(|(uid, slots)| {
            let classes: i64 = slots.values().sum();
            // Highest count; ties go to the earliest weekday/time.
            let (top, count) = slots
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| slot_sort_key(b.0).cmp(&slot_sort_key(a.0))))
                .map(|(k, v)| (k.clone(), *v))
                .unwrap_or_default();
            (
                uid,
                TeacherStats {
                    classes,
                    distinct_slots: slots.len() as i64,
                    top_slot: top,
                    top_slot_count: count,
                    top_slot_share: if classes > 0 { count as f64 / classes as f64 } else { 0.0 },
                },
            )
        })
        .collect()
}

fn totals(stats: &BTreeMap<i32, TeacherStats>) -> DraftTotals {
    let classes: i64 = stats.values().map(|s| s.classes).sum();
    let distinct_slots: i64 = stats.values().map(|s| s.distinct_slots).sum();
    DraftTotals {
        classes,
        distinct_slots,
        classes_per_slot: if distinct_slots > 0 { classes as f64 / distinct_slots as f64 } else { 0.0 },
    }
}

/// Pure diff of two drafts' shifts (reuses the candidate-validation slot
/// pairing, so co-teach rows keep their own identity).
pub fn diff_shifts(
    month: &str,
    a: &[RunShift],
    b: &[RunShift],
    names: &HashMap<i32, String>,
) -> (Vec<DraftSlotDiff>, Vec<TeacherConsistency>, DraftTotals, DraftTotals) {
    let cmp = compare_runs(month, a, b, names, &HashMap::new());
    let changes = cmp
        .changes
        .into_iter()
        .map(|c| {
            let kind = match c.kind.as_str() {
                "added" => "only_b",
                "removed" => "only_a",
                _ => match (c.class_before != c.class_after, c.teacher_before != c.teacher_after) {
                    (true, true) => "format_teacher",
                    (true, false) => "format",
                    _ => "teacher",
                },
            };
            DraftSlotDiff {
                date: c.date,
                weekday: c.weekday,
                start: c.start,
                class_a: c.class_before,
                class_b: c.class_after,
                teacher_a: c.teacher_before,
                teacher_b: c.teacher_after,
                kind: kind.to_string(),
            }
        })
        .collect();

    let (sa, sb) = (consistency_stats(a), consistency_stats(b));
    let mut uids: Vec<i32> = sa.keys().chain(sb.keys()).copied().collect();
    uids.sort_unstable();
    uids.dedup();
    let mut teachers: Vec<TeacherConsistency> = uids
        .into_iter()
        .map(|uid| TeacherConsistency {
            sling_user_id: uid,
            name: names.get(&uid).cloned().unwrap_or_else(|| format!("teacher {uid}")),
            a: sa.get(&uid).cloned(),
            b: sb.get(&uid).cloned(),
        })
        .collect();
    teachers.sort_by(|x, y| x.name.cmp(&y.name));
    (changes, teachers, totals(&sa), totals(&sb))
}

fn load_run_shifts(conn: &duckdb::Connection, proposal_id: i64) -> Result<Vec<RunShift>, String> {
    conn.prepare(
        "SELECT CAST(ps.shift_date AS VARCHAR), ps.start_time, ps.sling_position_id,
                COALESCE(pos.class_name, 'position ' || CAST(ps.sling_position_id AS VARCHAR)),
                ps.sling_user_id, COALESCE(ps.coteach_label, ''), ps.is_dropped
         FROM proposal_shifts ps
         LEFT JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
         WHERE ps.proposal_id = ?
         ORDER BY ps.shift_date, ps.start_time, ps.id",
    )
    .map_err(err)?
    .query_map(duckdb::params![proposal_id], |r| {
        Ok(RunShift {
            date: r.get(0)?,
            start: r.get(1)?,
            position_id: r.get(2)?,
            class_name: r.get(3)?,
            user_id: r.get(4)?,
            coteach_label: r.get(5)?,
            dropped: r.get(6)?,
        })
    })
    .map_err(err)?
    .collect::<Result<_, _>>()
    .map_err(err)
}

pub fn diff_impl(conn: &duckdb::Connection, a: i64, b: i64) -> Result<ProposalDiff, String> {
    let (ma, mb) = (month_of(conn, a)?, month_of(conn, b)?);
    if ma != mb {
        return Err(format!("drafts are for different months ({ma} vs {mb})"));
    }
    let names: HashMap<i32, String> = conn
        .prepare("SELECT sling_user_id, display_name FROM teachers")
        .map_err(err)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let (sa, sb) = (load_run_shifts(conn, a)?, load_run_shifts(conn, b)?);
    let (changes, teachers, totals_a, totals_b) = diff_shifts(&ma, &sa, &sb, &names);
    Ok(ProposalDiff {
        target_month: ma,
        a_id: a,
        b_id: b,
        a_name: draft_name(conn, a),
        b_name: draft_name(conn, b),
        changes,
        teachers,
        totals_a,
        totals_b,
    })
}

// ============================================================
// Tauri commands
// ============================================================

#[tauri::command(async)]
pub fn duplicate_proposal(
    db: State<'_, Db>,
    proposal_id: i64,
    name: Option<String>,
) -> Result<i64, String> {
    let mut conn = db.0.lock().map_err(err)?;
    duplicate_impl(&mut conn, proposal_id, name.as_deref())
}

#[tauri::command(async)]
pub fn rename_proposal(db: State<'_, Db>, proposal_id: i64, name: String) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    rename_impl(&conn, proposal_id, &name)
}

#[tauri::command(async)]
pub fn archive_proposal(db: State<'_, Db>, proposal_id: i64) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    set_archived_impl(&conn, proposal_id, true)
}

#[tauri::command(async)]
pub fn unarchive_proposal(db: State<'_, Db>, proposal_id: i64) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    set_archived_impl(&conn, proposal_id, false)
}

#[tauri::command(async)]
pub fn set_push_candidate(
    db: State<'_, Db>,
    target_month: String,
    proposal_id: i64,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    set_push_candidate_impl(&conn, &target_month, proposal_id)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(())
}

#[tauri::command(async)]
pub fn diff_proposals(db: State<'_, Db>, a: i64, b: i64) -> Result<ProposalDiff, String> {
    let conn = db.0.lock().map_err(err)?;
    diff_impl(&conn, a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> duckdb::Connection {
        let c = crate::db::open_in_memory().unwrap();
        crate::migrations::run(&c).unwrap();
        c.execute_batch(
            "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max) VALUES
               (501, 'Alex', 4, 5), (502, 'Kay', 4, 5);
             INSERT INTO positions (sling_position_id, class_name) VALUES (101, 'Classic'), (106, 'Focus');",
        )
        .unwrap();
        c
    }

    /// Insert a generated draft with the given shifts (date, start, pos, uid).
    fn generated(c: &duckdb::Connection, month: &str, shifts: &[(&str, &str, i32, Option<i32>)]) -> i64 {
        let id: i64 = c
            .query_row(
                "INSERT INTO proposals (target_month, algorithm_version, parameters, is_current)
                 VALUES (?, 'v9', '{}', TRUE) RETURNING id",
                duckdb::params![month],
                |r| r.get(0),
            )
            .unwrap();
        for (d, t, p, u) in shifts {
            c.execute(
                "INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time,
                     sling_position_id, sling_user_id, generation_reason, is_dropped)
                 VALUES (?, CAST(? AS DATE), ?, '23:59', ?, ?, 'test', ?)",
                duckdb::params![id, d, t, p, u, u.is_none()],
            )
            .unwrap();
        }
        record_generated(c, id, month, None).unwrap();
        id
    }

    #[test]
    fn generate_names_drafts_and_sets_push_draft_only_once() {
        let c = conn();
        let a = generated(&c, "2026-08", &[]);
        let b = generated(&c, "2026-08", &[]);
        assert_eq!(draft_name(&c, a), "Draft 1");
        assert_eq!(draft_name(&c, b), "Draft 2");
        // The second generate does NOT steal the push draft.
        assert_eq!(push_candidate(&c, "2026-08").unwrap(), Some(a));
        // A named generate; names are trimmed and validated.
        let id: i64 = c
            .query_row(
                "INSERT INTO proposals (target_month, algorithm_version, parameters) VALUES ('2026-08','v9','{}') RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        record_generated(&c, id, "2026-08", Some("  Consistent days ")).unwrap();
        assert_eq!(draft_name(&c, id), "Consistent days");
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"x".repeat(61)).is_err());
    }

    #[test]
    fn duplicate_copies_shifts_and_remaps_coteach_partner() {
        let mut c = conn();
        let src = generated(
            &c,
            "2026-08",
            &[("2026-08-03", "09:00", 101, Some(501)), ("2026-08-08", "10:00", 106, Some(501))],
        );
        // Link the two rows as co-teach partners (the legacy sibling-row shape)
        // and give one an edit, which must NOT be copied.
        let ids: Vec<i64> = c
            .prepare("SELECT id FROM proposal_shifts WHERE proposal_id = ? ORDER BY id")
            .unwrap()
            .query_map(duckdb::params![src], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        c.execute(
            "UPDATE proposal_shifts SET is_coteach = TRUE, coteach_label = 'Alex + Kay', coteach_partner_shift_id = ? WHERE id = ?",
            duckdb::params![ids[0], ids[1]],
        )
        .unwrap();
        c.execute(
            "INSERT INTO edits (proposal_shift_id, field, old_value, new_value) VALUES (?, 'sling_user_id', '502', '501')",
            duckdb::params![ids[0]],
        )
        .unwrap();

        let copy = duplicate_impl(&mut c, src, None).unwrap();
        assert_ne!(copy, src);
        assert_eq!(draft_name(&c, copy), "Copy of Draft 1");
        let (parent, from): (Option<i64>, String) = c
            .query_row(
                "SELECT parent_proposal_id, created_from FROM proposal_drafts WHERE proposal_id = ?",
                duckdb::params![copy],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((parent, from.as_str()), (Some(src), "duplicate"));

        let rows: Vec<(i64, Option<i64>, bool, Option<String>)> = c
            .prepare(
                "SELECT id, coteach_partner_shift_id, is_coteach, coteach_label
                 FROM proposal_shifts WHERE proposal_id = ? ORDER BY id",
            )
            .unwrap()
            .query_map(duckdb::params![copy], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| !ids.contains(&r.0)), "fresh ids");
        // Partner points at the COPY of the original partner, not the original.
        assert_eq!(rows[1].1, Some(rows[0].0));
        assert!(rows[1].2);
        assert_eq!(rows[1].3.as_deref(), Some("Alex + Kay"));

        // No edit history copied; the copy is not the push draft; is_current FALSE.
        let edits: i64 = c
            .query_row(
                "SELECT count(*) FROM edits e JOIN proposal_shifts ps ON ps.id = e.proposal_shift_id WHERE ps.proposal_id = ?",
                duckdb::params![copy],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(edits, 0);
        assert_eq!(push_candidate(&c, "2026-08").unwrap(), Some(src));
        let cur: bool = c
            .query_row("SELECT is_current FROM proposals WHERE id = ?", duckdb::params![copy], |r| r.get(0))
            .unwrap();
        assert!(!cur);

        // Editing the copy leaves the source untouched.
        c.execute(
            "UPDATE proposal_shifts SET sling_user_id = 502 WHERE id = ?",
            duckdb::params![rows[0].0],
        )
        .unwrap();
        let src_uid: i32 = c
            .query_row("SELECT sling_user_id FROM proposal_shifts WHERE id = ?", duckdb::params![ids[0]], |r| r.get(0))
            .unwrap();
        assert_eq!(src_uid, 501);
    }

    #[test]
    fn push_candidate_enforcement_archive_and_rename() {
        let c = conn();
        let a = generated(&c, "2026-08", &[]);
        let b = generated(&c, "2026-08", &[]);
        let other_month = generated(&c, "2026-09", &[]);

        ensure_push_candidate(&c, a).expect("push draft may push");
        let e = ensure_push_candidate(&c, b).unwrap_err();
        assert!(e.contains("not the push draft") && e.contains("Use for push"), "{e}");

        // Can't archive the push draft; can archive another; archived can't be pushed-for.
        assert!(set_archived_impl(&c, a, true).is_err());
        set_archived_impl(&c, b, true).unwrap();
        assert!(set_push_candidate_impl(&c, "2026-08", b).unwrap_err().contains("archived"));
        set_archived_impl(&c, b, false).unwrap();

        // Switching the push draft flips enforcement.
        set_push_candidate_impl(&c, "2026-08", b).unwrap();
        assert!(ensure_push_candidate(&c, a).is_err());
        ensure_push_candidate(&c, b).unwrap();
        // Idempotent, and month mismatch refused.
        set_push_candidate_impl(&c, "2026-08", b).unwrap();
        assert!(set_push_candidate_impl(&c, "2026-08", other_month).is_err());
        let n: i64 = c
            .query_row("SELECT count(*) FROM month_push_candidate WHERE target_month = '2026-08'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);

        rename_impl(&c, a, "Consistent days").unwrap();
        rename_impl(&c, a, "Consistent days").unwrap(); // no-op write
        assert_eq!(draft_name(&c, a), "Consistent days");
        assert!(rename_impl(&c, a, " ").is_err());
        assert!(ensure_push_candidate(&c, a).unwrap_err().contains("\"Consistent days\""));

        // A push on record for the other draft shows up as a warning.
        c.execute("INSERT INTO pushes (proposal_id) VALUES (?)", duckdb::params![a]).unwrap();
    }

    #[test]
    fn diff_and_consistency_metric() {
        let c = conn();
        // Mondays in Aug 2026: 3, 10, 17, 24, 31. Tuesdays: 4, 11, 18, 25.
        // Draft A rotates Alex between Mon 09:00 and Tue 17:30; B keeps Alex
        // on Mon 09:00 and Kay on Tue 17:30 all month.
        let a = generated(
            &c,
            "2026-08",
            &[
                ("2026-08-03", "09:00", 101, Some(501)),
                ("2026-08-04", "17:30", 101, Some(502)),
                ("2026-08-10", "09:00", 101, Some(502)),
                ("2026-08-11", "17:30", 101, Some(501)),
                ("2026-08-17", "09:00", 101, None),
            ],
        );
        let b = generated(
            &c,
            "2026-08",
            &[
                ("2026-08-03", "09:00", 101, Some(501)),
                ("2026-08-04", "17:30", 101, Some(502)),
                ("2026-08-10", "09:00", 101, Some(501)),
                ("2026-08-11", "17:30", 106, Some(502)),
                ("2026-08-17", "09:00", 101, Some(501)),
            ],
        );
        let d = diff_impl(&c, a, b).unwrap();
        assert_eq!((d.a_name.as_str(), d.b_name.as_str()), ("Draft 1", "Draft 2"));
        let kinds: Vec<(&str, &str, &str)> = d
            .changes
            .iter()
            .map(|x| (x.date.as_str(), x.start.as_str(), x.kind.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("2026-08-10", "09:00", "teacher"),
                ("2026-08-11", "17:30", "format_teacher"),
                ("2026-08-17", "09:00", "teacher"),
            ]
        );
        assert_eq!(d.changes[0].weekday, "Mon");
        assert_eq!(d.changes[2].teacher_a.as_deref(), Some("Dropped"));
        assert_eq!(d.changes[2].teacher_b.as_deref(), Some("Alex"));

        let alex = d.teachers.iter().find(|t| t.sling_user_id == 501).unwrap();
        let (aa, ab) = (alex.a.as_ref().unwrap(), alex.b.as_ref().unwrap());
        assert_eq!((aa.classes, aa.distinct_slots, aa.top_slot_count), (2, 2, 1));
        assert_eq!(aa.top_slot, "Mon 09:00"); // tie -> earliest weekday
        assert!((aa.top_slot_share - 0.5).abs() < 1e-9);
        assert_eq!((ab.classes, ab.distinct_slots, ab.top_slot.as_str()), (3, 1, "Mon 09:00"));
        assert!((ab.top_slot_share - 1.0).abs() < 1e-9);
        // Totals: A = 4 classes over 4 teacher-slots; B = 5 over 2.
        assert_eq!((d.totals_a.classes, d.totals_a.distinct_slots), (4, 4));
        assert_eq!((d.totals_b.classes, d.totals_b.distinct_slots), (5, 2));
        assert!(d.totals_b.classes_per_slot > d.totals_a.classes_per_slot);

        // Identical drafts: no changes. Cross-month compare refused.
        assert!(diff_impl(&c, a, a).unwrap().changes.is_empty());
        let sep = generated(&c, "2026-09", &[]);
        assert!(diff_impl(&c, a, sep).is_err());
    }
}
