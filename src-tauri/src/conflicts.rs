// Re-validate an existing draft against the latest pulled Sling data
// (availability refresh, migration 0013) instead of regenerating it.
//
// Checks per shift: teacher blocked (Sling `availability` events are
// BLOCKED time — the naming is backward), on leave, deactivated, not
// qualified, over their weekly cap, and unassigned slots. These mirror the
// frontend issue queue (src/lib/issues.ts) but run against the database so
// a refresh can report exactly what it broke; the proposer's scoring rules
// (variety, rankings, rotation) are deliberately NOT re-run here.
//
// Time handling: availability_blocks are TIMESTAMPTZ (absolute instants);
// shifts are studio-local wall-clock strings. The shift is converted to an
// instant with US Central rules (CST -06:00 / CDT -05:00, switching on the
// second Sunday of March and the first Sunday of November at 02:00 local),
// so an overlap test is right on both sides of a DST change — unlike a
// fixed -05:00 offset.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use serde::Serialize;
use tauri::State;

use crate::db::Db;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// nth (1-based) `weekday` of a month.
fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: u32) -> NaiveDate {
    NaiveDate::from_weekday_of_month_opt(year, month, weekday, n as u8).expect("valid nth weekday")
}

/// US Central UTC offset in minutes (-300 CDT / -360 CST) for a local
/// wall-clock time. The repeated 01:00–02:00 hour in November resolves to
/// CDT (the first occurrence); no class runs then.
pub fn central_offset_minutes(local: NaiveDateTime) -> i32 {
    let y = local.date().year();
    let two = NaiveTime::from_hms_opt(2, 0, 0).expect("02:00");
    let dst_start = nth_weekday(y, 3, Weekday::Sun, 2).and_time(two);
    let dst_end = nth_weekday(y, 11, Weekday::Sun, 1).and_time(two);
    // Local 01:00–02:00 on the fall-back day occurs twice; this range
    // resolves it (and the skipped spring hour) to CDT.
    if local >= dst_start && local < dst_end {
        -300
    } else {
        -360
    }
}

/// Studio-local "YYYY-MM-DD" + "HH:MM" → Unix seconds.
pub fn local_to_epoch(date: &str, hhmm: &str) -> Option<i64> {
    let local = NaiveDateTime::parse_from_str(&format!("{date} {hhmm}"), "%Y-%m-%d %H:%M").ok()?;
    let off = central_offset_minutes(local) as i64;
    Some(local.and_utc().timestamp() - off * 60)
}

/// Unix seconds → studio-local wall clock.
pub fn epoch_to_local(epoch: i64) -> Option<NaiveDateTime> {
    let utc = chrono::DateTime::from_timestamp(epoch, 0)?.naive_utc();
    let cdt = utc - chrono::Duration::minutes(300);
    if central_offset_minutes(cdt) == -300 {
        Some(cdt)
    } else {
        Some(utc - chrono::Duration::minutes(360))
    }
}

#[derive(Debug, Clone)]
pub struct ConflictShift {
    pub id: i64,
    pub date: String,
    pub start: String,
    pub end: String,
    pub class_name: String,
    pub position_id: i64,
    /// Everyone teaching it (two for a co-teach row).
    pub user_ids: Vec<i64>,
    pub is_dropped: bool,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub user_id: i64,
    pub source: String, // "availability" (= blocked) | "leave"
    pub start: i64,     // Unix seconds
    pub end: i64,
}

#[derive(Debug, Clone)]
pub struct TeacherInfo {
    pub name: String,
    pub weekly_max: i64,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DraftConflict {
    pub proposal_shift_id: i64,
    pub shift_date: String,
    pub start_time: String,
    pub end_time: String,
    pub class_name: String,
    pub sling_user_id: Option<i64>,
    pub teacher_name: Option<String>,
    /// blocked | leave | teacher_inactive | not_qualified | over_cap | unassigned
    pub kind: String,
    pub message: String,
}

fn fmt_block(b: &Block, shift_date: &str) -> String {
    let (Some(s), Some(e)) = (epoch_to_local(b.start), epoch_to_local(b.end)) else {
        return String::new();
    };
    let same_day = s.date() == e.date() && s.date().to_string() == shift_date;
    if same_day {
        format!("{}–{}", s.format("%H:%M"), e.format("%H:%M"))
    } else {
        format!("{} → {}", s.format("%b %-d %H:%M"), e.format("%b %-d %H:%M"))
    }
}

pub fn detect_conflicts(
    shifts: &[ConflictShift],
    blocks: &[Block],
    teachers: &HashMap<i64, TeacherInfo>,
    quals: &HashSet<(i64, i64)>,
) -> Vec<DraftConflict> {
    let mut out = Vec::new();
    let name = |uid: i64| teachers.get(&uid).map(|t| t.name.clone()).unwrap_or_else(|| format!("user {uid}"));
    let mk = |s: &ConflictShift, uid: Option<i64>, kind: &str, message: String| DraftConflict {
        proposal_shift_id: s.id,
        shift_date: s.date.clone(),
        start_time: s.start.clone(),
        end_time: s.end.clone(),
        class_name: s.class_name.clone(),
        sling_user_id: uid,
        teacher_name: uid.map(name),
        kind: kind.to_string(),
        message,
    };

    // (user, iso year, iso week) -> (count, latest shift)
    let mut weekly: HashMap<(i64, i32, u32), (i64, &ConflictShift)> = HashMap::new();

    for s in shifts.iter().filter(|s| !s.is_dropped) {
        if s.user_ids.is_empty() {
            out.push(mk(s, None, "unassigned", format!("{} {} has no teacher", s.start, s.class_name)));
            continue;
        }
        let (Some(start), Some(end)) = (local_to_epoch(&s.date, &s.start), local_to_epoch(&s.date, &s.end)) else {
            continue;
        };
        for &uid in &s.user_ids {
            for b in blocks.iter().filter(|b| b.user_id == uid && b.start < end && b.end > start) {
                let (kind, what) = if b.source == "leave" {
                    ("leave", "is on leave")
                } else {
                    ("blocked", "is marked unavailable")
                };
                out.push(mk(
                    s,
                    Some(uid),
                    kind,
                    format!("{} {what} ({}) — overlaps {} {}", name(uid), fmt_block(b, &s.date), s.start, s.class_name),
                ));
            }
            match teachers.get(&uid) {
                Some(t) if !t.active => {
                    out.push(mk(s, Some(uid), "teacher_inactive", format!("{} is deactivated in Sling", t.name)))
                }
                None => out.push(mk(s, Some(uid), "teacher_inactive", format!("{} is not on the roster", name(uid)))),
                _ => {}
            }
            if !quals.contains(&(uid, s.position_id)) {
                out.push(mk(
                    s,
                    Some(uid),
                    "not_qualified",
                    format!("{} is not qualified for {} in Sling", name(uid), s.class_name),
                ));
            }
            if let Ok(d) = NaiveDate::parse_from_str(&s.date, "%Y-%m-%d") {
                let w = d.iso_week();
                let e = weekly.entry((uid, w.year(), w.week())).or_insert((0, s));
                e.0 += 1;
                if (s.date.as_str(), s.start.as_str()) > (e.1.date.as_str(), e.1.start.as_str()) {
                    e.1 = s;
                }
            }
        }
    }

    let mut caps: Vec<_> = weekly.into_iter().collect();
    caps.sort_by(|a, b| (a.1 .1.date.as_str(), a.0 .0).cmp(&(b.1 .1.date.as_str(), b.0 .0)));
    for ((uid, _, _), (count, s)) in caps {
        if let Some(t) = teachers.get(&uid) {
            if count > t.weekly_max {
                out.push(mk(
                    s,
                    Some(uid),
                    "over_cap",
                    format!("{} is over their weekly cap ({count} / {})", t.name, t.weekly_max),
                ));
            }
        }
    }

    out.sort_by(|a, b| {
        (a.shift_date.as_str(), a.start_time.as_str(), a.proposal_shift_id)
            .cmp(&(b.shift_date.as_str(), b.start_time.as_str(), b.proposal_shift_id))
    });
    out
}

/// Load a draft + roster + blocks, detect conflicts, and record the check
/// (draft_checks) so the draft is no longer "stale" against this data.
pub fn check_impl(conn: &duckdb::Connection, proposal_id: i64) -> Result<Vec<DraftConflict>, String> {
    let month: String = conn
        .query_row(
            "SELECT target_month FROM proposals WHERE id = ?",
            duckdb::params![proposal_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("draft {proposal_id} not found: {e:#}"))?;

    let teachers: HashMap<i64, TeacherInfo> = conn
        .prepare("SELECT sling_user_id, display_name, weekly_max, active FROM teachers")
        .map_err(err)?
        .query_map([], |r| {
            Ok((
                r.get::<_, i32>(0)? as i64,
                TeacherInfo { name: r.get(1)?, weekly_max: r.get::<_, i32>(2)? as i64, active: r.get(3)? },
            ))
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let by_name: HashMap<String, i64> = teachers.iter().map(|(id, t)| (t.name.clone(), *id)).collect();
    let quals: HashSet<(i64, i64)> = conn
        .prepare("SELECT sling_user_id, sling_position_id FROM teacher_qualifications WHERE NOT is_blocklisted")
        .map_err(err)?
        .query_map([], |r| Ok((r.get::<_, i32>(0)? as i64, r.get::<_, i32>(1)? as i64)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;

    let shifts: Vec<ConflictShift> = conn
        .prepare(
            "SELECT ps.id, CAST(ps.shift_date AS VARCHAR), ps.start_time, ps.end_time, pos.class_name,
                    ps.sling_position_id, ps.sling_user_id, ps.is_coteach, ps.coteach_label, ps.is_dropped
             FROM proposal_shifts ps
             JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
             WHERE ps.proposal_id = ?
             ORDER BY ps.shift_date, ps.start_time",
        )
        .map_err(err)?
        .query_map(duckdb::params![proposal_id], |r| {
            let uid: Option<i32> = r.get(6)?;
            let is_coteach: bool = r.get(7)?;
            let label: Option<String> = r.get(8)?;
            // Co-teach rows name both teachers in the label (see
            // sling::build_push_specs); fall back to the primary uid.
            let mut user_ids: Vec<i64> = if is_coteach {
                label
                    .as_deref()
                    .unwrap_or("")
                    .split(" + ")
                    .filter_map(|n| by_name.get(n.trim()).copied())
                    .collect()
            } else {
                Vec::new()
            };
            if user_ids.is_empty() {
                user_ids.extend(uid.map(|u| u as i64));
            }
            Ok(ConflictShift {
                id: r.get(0)?,
                date: r.get(1)?,
                start: r.get(2)?,
                end: r.get(3)?,
                class_name: r.get(4)?,
                position_id: r.get::<_, i32>(5)? as i64,
                user_ids,
                is_dropped: r.get(9)?,
            })
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;

    // Blocks overlapping the month, padded a day each side (a leave that
    // starts in the previous month still blocks its first days).
    // (Compared as epoch seconds: the bundled DuckDB has no ICU, so
    // TIMESTAMPTZ ± INTERVAL doesn't bind.)
    let first = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").map_err(err)?;
    let lo = local_to_epoch(&(first - chrono::Duration::days(1)).to_string(), "00:00").ok_or("bad month")?;
    let hi = local_to_epoch(&(first + chrono::Duration::days(32)).to_string(), "00:00").ok_or("bad month")?;
    let blocks: Vec<Block> = conn
        .prepare(
            "SELECT sling_user_id, source,
                    CAST(epoch(starts_at) AS BIGINT), CAST(epoch(ends_at) AS BIGINT)
             FROM availability_blocks
             WHERE epoch(starts_at) <= ? AND epoch(ends_at) >= ?",
        )
        .map_err(err)?
        .query_map(duckdb::params![hi, lo], |r| {
            Ok(Block {
                user_id: r.get::<_, i32>(0)? as i64,
                source: r.get(1)?,
                start: r.get(2)?,
                end: r.get(3)?,
            })
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;

    let conflicts = detect_conflicts(&shifts, &blocks, &teachers, &quals);
    conn.execute(
        "INSERT OR REPLACE INTO draft_checks (proposal_id, checked_at) VALUES (?, now())",
        duckdb::params![proposal_id],
    )
    .map_err(err)?;
    Ok(conflicts)
}

/// Re-validate a draft against the latest pulled availability/roster and
/// mark it checked (clears the stale banner without regenerating).
#[tauri::command]
pub fn check_draft_conflicts(db: State<'_, Db>, proposal_id: i64) -> Result<Vec<DraftConflict>, String> {
    let conn = db.0.lock().map_err(err)?;
    check_impl(&conn, proposal_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }
    fn at(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }

    #[test]
    fn central_offset_follows_us_dst_rules() {
        // 2026: DST starts Sun Mar 8, ends Sun Nov 1 (02:00 local).
        assert_eq!(central_offset_minutes(dt("2026-03-08 01:59")), -360);
        assert_eq!(central_offset_minutes(dt("2026-03-08 03:00")), -300);
        assert_eq!(central_offset_minutes(dt("2026-07-01 09:00")), -300);
        assert_eq!(central_offset_minutes(dt("2026-11-01 01:30")), -300);
        assert_eq!(central_offset_minutes(dt("2026-11-01 05:45")), -360);
        assert_eq!(central_offset_minutes(dt("2026-12-15 09:00")), -360);
        // 2027: Mar 14 / Nov 7.
        assert_eq!(central_offset_minutes(dt("2027-03-13 09:00")), -360);
        assert_eq!(central_offset_minutes(dt("2027-03-14 09:00")), -300);
        assert_eq!(central_offset_minutes(dt("2027-11-06 09:00")), -300);
        assert_eq!(central_offset_minutes(dt("2027-11-07 09:00")), -360);
    }

    #[test]
    fn local_epoch_round_trips_across_dst() {
        assert_eq!(local_to_epoch("2026-10-31", "05:45"), Some(at("2026-10-31T05:45:00-05:00")));
        assert_eq!(local_to_epoch("2026-11-01", "05:45"), Some(at("2026-11-01T05:45:00-06:00")));
        assert_eq!(local_to_epoch("2026-03-09", "05:45"), Some(at("2026-03-09T05:45:00-05:00")));
        for (d, t) in [("2026-11-01", "05:45"), ("2026-03-08", "09:00"), ("2026-06-15", "17:30")] {
            let e = local_to_epoch(d, t).unwrap();
            assert_eq!(epoch_to_local(e).unwrap(), dt(&format!("{d} {t}")));
        }
    }

    fn shift(id: i64, date: &str, start: &str, end: &str, users: &[i64]) -> ConflictShift {
        ConflictShift {
            id,
            date: date.into(),
            start: start.into(),
            end: end.into(),
            class_name: "Classic".into(),
            position_id: 1,
            user_ids: users.to_vec(),
            is_dropped: false,
        }
    }
    fn roster() -> (HashMap<i64, TeacherInfo>, HashSet<(i64, i64)>) {
        let mut t = HashMap::new();
        t.insert(1, TeacherInfo { name: "Alex".into(), weekly_max: 2, active: true });
        t.insert(2, TeacherInfo { name: "Kay".into(), weekly_max: 5, active: true });
        t.insert(3, TeacherInfo { name: "Cee".into(), weekly_max: 5, active: false });
        let q = [(1, 1), (2, 1), (3, 1)].into_iter().collect();
        (t, q)
    }
    fn block(user: i64, source: &str, from: &str, to: &str) -> Block {
        Block { user_id: user, source: source.into(), start: at(from), end: at(to) }
    }

    #[test]
    fn detects_blocked_and_leave_with_exact_edges() {
        let (t, q) = roster();
        let shifts = [
            shift(10, "2026-11-02", "05:45", "06:45", &[1]),
            shift(11, "2026-11-02", "09:00", "10:00", &[1]),
            shift(12, "2026-11-03", "09:00", "10:00", &[2]),
        ];
        let blocks = [
            // Blocked (Sling `availability` = BLOCKED time) 05:00–06:00 CST.
            block(1, "availability", "2026-11-02T05:00:00-06:00", "2026-11-02T06:00:00-06:00"),
            // Ends exactly when the 09:00 class starts → no overlap.
            block(1, "availability", "2026-11-02T08:00:00-06:00", "2026-11-02T09:00:00-06:00"),
            // Multi-day leave.
            block(2, "leave", "2026-11-03T00:00:00-06:00", "2026-11-05T00:00:00-06:00"),
        ];
        let c = detect_conflicts(&shifts, &blocks, &t, &q);
        let got: Vec<(i64, &str)> = c.iter().map(|c| (c.proposal_shift_id, c.kind.as_str())).collect();
        assert_eq!(got, vec![(10, "blocked"), (12, "leave")]);
        assert!(c[0].message.contains("Alex is marked unavailable (05:00–06:00)"), "{}", c[0].message);
        assert!(c[1].message.contains("on leave"), "{}", c[1].message);
    }

    #[test]
    fn dst_boundary_uses_the_right_offset() {
        let (t, q) = roster();
        // Sun Nov 1 2026 is fall-back day: a 05:45 class is 11:45Z, not 10:45Z.
        let shifts = [shift(10, "2026-11-01", "05:45", "06:45", &[1])];
        // A block 10:00Z–11:00Z would overlap only under a fixed -05:00.
        let early = [block(1, "availability", "2026-11-01T10:00:00Z", "2026-11-01T11:00:00Z")];
        assert!(detect_conflicts(&shifts, &early, &t, &q).is_empty());
        let real = [block(1, "availability", "2026-11-01T05:30:00-06:00", "2026-11-01T06:00:00-06:00")];
        assert_eq!(detect_conflicts(&shifts, &real, &t, &q).len(), 1);
        // Spring forward: Mon Mar 9 2026 05:45 is CDT.
        let shifts = [shift(11, "2026-03-09", "05:45", "06:45", &[1])];
        let b = [block(1, "leave", "2026-03-09T05:00:00-05:00", "2026-03-09T05:50:00-05:00")];
        assert_eq!(detect_conflicts(&shifts, &b, &t, &q).len(), 1);
    }

    #[test]
    fn roster_rules_and_coteach() {
        let (t, q) = roster();
        let mut dropped = shift(20, "2026-11-04", "09:00", "10:00", &[1]);
        dropped.is_dropped = true;
        let mut unqualified = shift(21, "2026-11-04", "17:30", "18:30", &[2]);
        unqualified.position_id = 9;
        let shifts = [
            shift(10, "2026-11-02", "09:00", "10:00", &[1]),
            shift(11, "2026-11-03", "09:00", "10:00", &[1]),
            shift(12, "2026-11-05", "09:00", "10:00", &[1, 3]), // co-teach, 3rd for Alex; Cee inactive
            dropped,
            unqualified,
            shift(22, "2026-11-06", "09:00", "10:00", &[]),
        ];
        // Co-teach partner blocked.
        let blocks = [block(3, "availability", "2026-11-05T09:30:00-06:00", "2026-11-05T12:00:00-06:00")];
        let c = detect_conflicts(&shifts, &blocks, &t, &q);
        let got: Vec<(i64, &str)> = c.iter().map(|c| (c.proposal_shift_id, c.kind.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (21, "not_qualified"),
                (12, "blocked"),
                (12, "teacher_inactive"),
                (12, "over_cap"),
                (22, "unassigned"),
            ]
        );
        assert_eq!(c[1].sling_user_id, Some(3));
        assert!(c[3].message.contains("(3 / 2)"), "{}", c[3].message);
    }

    #[test]
    fn check_impl_reads_timestamptz_blocks_and_clears_staleness() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max)
               VALUES (1, 'Alex', 3, 5), (2, 'Kay', 3, 5);
             INSERT INTO positions (sling_position_id, class_name) VALUES (7, 'Classic');
             INSERT INTO teacher_qualifications (sling_user_id, sling_position_id) VALUES (1, 7), (2, 7);
             INSERT INTO proposals (id, target_month, algorithm_version, parameters, generated_at)
               VALUES (1, '2026-11', 'v9', '{}', TIMESTAMPTZ '2026-01-20 10:00:00+00');
             INSERT INTO proposal_shifts (id, proposal_id, shift_date, start_time, end_time,
                 sling_position_id, sling_user_id, generation_reason, is_coteach, coteach_label) VALUES
               (10, 1, DATE '2026-11-02', '05:45', '06:45', 7, 1, 'r', FALSE, NULL),
               (11, 1, DATE '2026-11-03', '09:00', '10:00', 7, 1, 'r', TRUE, 'Alex + Kay');
             INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at) VALUES
               (1, 'availability', CAST('2026-11-02T05:00:00-06:00' AS TIMESTAMPTZ), CAST('2026-11-02T06:00:00-06:00' AS TIMESTAMPTZ)),
               (1, 'availability', CAST('2026-11-02T06:45:00-06:00' AS TIMESTAMPTZ), CAST('2026-11-02T08:00:00-06:00' AS TIMESTAMPTZ)),
               (2, 'leave', CAST('2026-11-03T00:00:00-06:00' AS TIMESTAMPTZ), CAST('2026-11-04T00:00:00-06:00' AS TIMESTAMPTZ));
             INSERT INTO month_pulls (target_month, pulled_at, user_count, qual_count, availability_count, external_shift_count)
               VALUES ('2026-11', TIMESTAMPTZ '2026-01-25 10:00:00+00', 2, 2, 3, 0);",
        )
        .unwrap();
        let (stale, pulled, checked) = crate::commands::staleness(&conn, 1).unwrap();
        assert!(stale && pulled.is_some() && checked.is_none());

        let c = check_impl(&conn, 1).unwrap();
        let got: Vec<(i64, &str, Option<i64>)> =
            c.iter().map(|c| (c.proposal_shift_id, c.kind.as_str(), c.sling_user_id)).collect();
        assert_eq!(got, vec![(10, "blocked", Some(1)), (11, "leave", Some(2))]);

        let (stale, _, checked) = crate::commands::staleness(&conn, 1).unwrap();
        assert!(!stale && checked.is_some());
        // A newer pull makes it stale again.
        conn.execute("UPDATE month_pulls SET pulled_at = TIMESTAMPTZ '2099-01-01 00:00:00+00' WHERE target_month = '2026-11'", [])
            .unwrap();
        assert!(crate::commands::staleness(&conn, 1).unwrap().0);
    }
}
