// Teacher availability: blocks, studio hours and computed windows.
//
//   1. Blocks (`availability_blocks`) come from the Sling calendar feed only,
//      written by commands::write_month_events: type `availability` (BLOCKED
//      time — Sling's naming is backward, see CLAUDE.md) and `leave`. The
//      feed already carries recurring unavailability expanded per date (each
//      occurrence has an `rrule` and an id like "5207099321:2026-10-07").
//      Sling's `GET /availability?userId=…` "availability sets" endpoint
//      returns only set headers with no times or durations, so it is not
//      used (its `sling_availability_sets` table from migration 0014 stays,
//      unused — migrations are forward-only).
//   2. Studio hours (`studio_hours`) — per-weekday open/close.
//   3. Computed AVAILABLE windows (`teacher_availability_windows`): each
//      date's studio hours, widened to any class slot outside them, minus
//      every block. Blocks stay the source of truth (propose.py and
//      conflicts.rs decide from blocks); the windows are the same facts
//      turned inside out for the Availability view, the day editor and the
//      Claude editor. `windows_and_blocks_agree_for_every_slot` pins the two
//      together.
//
// Every source in availability_blocks is blocked time.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::db::Db;
use crate::sling::{studio_local, STUDIO_TZ};

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

// ============================================================
// Block sources
// ============================================================

// `availability` — blocked time from the calendar feed (the Sling event
// type, stored verbatim by commands::write_month_events), one-off or an
// occurrence of a recurring entry.
/// Approved time off from the calendar feed (Sling type `leave`).
pub const SOURCE_LEAVE: &str = "leave";
/// Legacy: blocks expanded from a pending availability set (no longer
/// written; old rows vanish when their month is re-pulled). Still blocked.
pub const SOURCE_SET_PENDING: &str = "availability_set_pending";

/// The event `type` propose.py sees for a block source. It only knows
/// 'leave' and 'availability' (both blocked); any other (legacy or unknown)
/// source is presented as 'availability' so every script version treats it
/// as blocked.
pub fn propose_event_type(source: &str) -> &'static str {
    if source == SOURCE_LEAVE {
        "leave"
    } else {
        "availability"
    }
}

/// A Sling date-time as studio-local wall-clock time. Accepts an offset
/// (`…-05:00`, `…-0500`, `Z`), fractional seconds, a space instead of `T`, a
/// naive local time, or a bare date (= local midnight).
pub fn parse_local(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim().replace(' ', "T");
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&s) {
        return Some(dt.with_timezone(&STUDIO_TZ).naive_local());
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%dT%H:%M%z"] {
        if let Ok(dt) = chrono::DateTime::parse_from_str(&s, fmt) {
            return Some(dt.with_timezone(&STUDIO_TZ).naive_local());
        }
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&s, fmt) {
            return Some(dt);
        }
    }
    NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok().map(|d| d.and_time(NaiveTime::MIN))
}

/// How many calendar days of `target_month` a block from `dtstart` to
/// `dtend` (Sling timestamps) touches, studio local. 0 if unreadable.
pub fn days_in_month(dtstart: &str, dtend: &str, target_month: &str) -> i64 {
    let (Some(start), Some(end), Ok((first, next))) = (parse_local(dtstart), parse_local(dtend), month_bounds(target_month))
    else {
        return 0;
    };
    let last = if end > start { (end - chrono::Duration::seconds(1)).date() } else { start.date() };
    let from = start.date().max(first);
    let to = last.min(next.pred_opt().unwrap_or(next));
    ((to - from).num_days() + 1).max(0)
}

fn month_bounds(target_month: &str) -> Result<(NaiveDate, NaiveDate), String> {
    let first = NaiveDate::parse_from_str(&format!("{target_month}-01"), "%Y-%m-%d")
        .map_err(|_| format!("bad target_month: {target_month}"))?;
    let next = first.checked_add_months(chrono::Months::new(1)).ok_or("invalid month")?;
    Ok((first, next))
}

// ============================================================
// Studio hours
// ============================================================

/// One weekday's studio hours. `weekday`: 0 = Monday … 6 = Sunday.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DayHours {
    pub weekday: u8,
    pub closed: bool,
    pub open: Option<String>,
    pub close: Option<String>,
}

fn hhmm_minutes(s: &str) -> Option<i32> {
    let t = NaiveTime::parse_from_str(s, "%H:%M").ok()?;
    use chrono::Timelike;
    Some((t.hour() * 60 + t.minute()) as i32)
}

fn minutes_hhmm(m: i32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// The saved studio hours; empty = never set (hours are then derived from
/// the class slots themselves).
pub fn load_studio_hours(conn: &duckdb::Connection) -> Result<Vec<DayHours>, String> {
    conn.prepare("SELECT weekday, closed, open_time, close_time FROM studio_hours ORDER BY weekday")
        .map_err(err)?
        .query_map([], |r| {
            Ok(DayHours { weekday: r.get::<_, i32>(0)? as u8, closed: r.get(1)?, open: r.get(2)?, close: r.get(3)? })
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)
}

/// Validate + replace the studio hours (DELETE + INSERT). An empty list
/// clears them (back to "derive from class slots").
pub fn save_studio_hours(conn: &duckdb::Connection, days: &[DayHours]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for d in days {
        if d.weekday > 6 || !seen.insert(d.weekday) {
            return Err(format!("invalid or repeated weekday {}", d.weekday));
        }
        if !d.closed {
            let open = d.open.as_deref().and_then(hhmm_minutes);
            let close = d.close.as_deref().and_then(hhmm_minutes);
            match (open, close) {
                (Some(o), Some(c)) if o < c => {}
                _ => {
                    return Err(format!(
                        "{}: opening time must be before closing time (HH:MM)",
                        WEEKDAY_NAMES[d.weekday as usize]
                    ))
                }
            }
        }
    }
    conn.execute("DELETE FROM studio_hours", []).map_err(err)?;
    for d in days {
        let (open, close) = if d.closed { (None, None) } else { (d.open.clone(), d.close.clone()) };
        conn.execute(
            "INSERT INTO studio_hours (weekday, closed, open_time, close_time) VALUES (?, ?, ?, ?)",
            duckdb::params![d.weekday as i32, d.closed, open, close],
        )
        .map_err(err)?;
    }
    Ok(())
}

const WEEKDAY_NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// A class slot: studio-local date + "HH:MM" start/end.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Slot {
    pub date: String,
    pub start: String,
    pub end: String,
}

/// Class slots between two months (inclusive): Sling shifts the app has
/// pulled plus the non-dropped shifts of non-archived drafts.
fn load_slots(conn: &duckdb::Connection, from_month: &str, to_month: &str) -> Result<Vec<Slot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT CAST(shift_date AS VARCHAR), start_time, end_time
             FROM external_sling_shifts WHERE target_month >= ? AND target_month <= ?
             UNION
             SELECT CAST(ps.shift_date AS VARCHAR), ps.start_time, ps.end_time
             FROM proposal_shifts ps
             JOIN proposals p ON p.id = ps.proposal_id
             LEFT JOIN proposal_drafts d ON d.proposal_id = p.id
             WHERE p.target_month >= ? AND p.target_month <= ?
               AND NOT ps.is_dropped AND NOT coalesce(d.archived, FALSE)",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(duckdb::params![from_month, to_month, from_month, to_month], |r| {
            Ok(Slot { date: r.get(0)?, start: r.get(1)?, end: r.get(2)? })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

fn weekday_index(date: &str) -> Option<u8> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok().map(|d| d.weekday().num_days_from_monday() as u8)
}

/// Hours implied by a set of class slots: per weekday, the earliest start
/// and the latest end; a weekday with no slot is closed. Always 7 entries.
pub fn hours_from_slots(slots: &[Slot]) -> Vec<DayHours> {
    let mut span: [Option<(i32, i32)>; 7] = [None; 7];
    for s in slots {
        let (Some(wd), Some(a), Some(b)) = (weekday_index(&s.date), hhmm_minutes(&s.start), hhmm_minutes(&s.end)) else {
            continue;
        };
        if b <= a {
            continue;
        }
        let cur = &mut span[wd as usize];
        *cur = Some(match *cur {
            Some((lo, hi)) => (lo.min(a), hi.max(b)),
            None => (a, b),
        });
    }
    (0..7u8)
        .map(|wd| match span[wd as usize] {
            Some((lo, hi)) => {
                DayHours { weekday: wd, closed: false, open: Some(minutes_hhmm(lo)), close: Some(minutes_hhmm(hi)) }
            }
            None => DayHours { weekday: wd, closed: true, open: None, close: None },
        })
        .collect()
}

fn month_minus(target_month: &str, months: u32) -> Result<String, String> {
    let (first, _) = month_bounds(target_month)?;
    Ok(first.checked_sub_months(chrono::Months::new(months)).ok_or("invalid month")?.format("%Y-%m").to_string())
}

/// "Suggest from schedule": hours from the class slots of the last three
/// months through every later month the app knows about.
pub fn suggest_studio_hours(conn: &duckdb::Connection, current_month: &str) -> Result<Vec<DayHours>, String> {
    let slots = load_slots(conn, &month_minus(current_month, 3)?, "9999-12")?;
    Ok(hours_from_slots(&slots))
}

// ============================================================
// Available windows
// ============================================================

/// A stretch of one date a teacher is available for, studio local.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Window {
    pub sling_user_id: i32,
    pub date: String,
    pub start: String,
    pub end: String,
}

/// The span of one date that counts as schedulable.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DayRange {
    pub date: String,
    pub open: String,
    pub close: String,
    /// A class slot on this date reached outside the normal hours.
    pub widened: bool,
}

/// A block as an absolute interval (Unix seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockSpan {
    pub user_id: i32,
    pub start: i64,
    pub end: i64,
}

/// Each date's schedulable span: that weekday's hours, widened to cover
/// every class slot on the date (the studio occasionally holds a class
/// outside its normal range). A closed day with a class is as long as its
/// classes; a closed day with none has no span.
pub fn day_ranges(first: NaiveDate, next_first: NaiveDate, hours: &[DayHours], slots: &[Slot]) -> Vec<DayRange> {
    let by_weekday: HashMap<u8, &DayHours> = hours.iter().map(|h| (h.weekday, h)).collect();
    let mut slot_span: HashMap<&str, (i32, i32)> = HashMap::new();
    for s in slots {
        let (Some(a), Some(b)) = (hhmm_minutes(&s.start), hhmm_minutes(&s.end)) else { continue };
        if b <= a {
            continue;
        }
        slot_span
            .entry(s.date.as_str())
            .and_modify(|(lo, hi)| {
                *lo = (*lo).min(a);
                *hi = (*hi).max(b);
            })
            .or_insert((a, b));
    }
    let mut out = Vec::new();
    let mut day = first;
    while day < next_first {
        let date = day.to_string();
        let base = by_weekday
            .get(&(day.weekday().num_days_from_monday() as u8))
            .filter(|h| !h.closed)
            .and_then(|h| Some((hhmm_minutes(h.open.as_deref()?)?, hhmm_minutes(h.close.as_deref()?)?)));
        let range = match (base, slot_span.get(date.as_str())) {
            (Some((o, c)), Some(&(lo, hi))) => Some((o.min(lo), c.max(hi), lo < o || hi > c)),
            (Some((o, c)), None) => Some((o, c, false)),
            (None, Some(&(lo, hi))) => Some((lo, hi, true)),
            (None, None) => None,
        };
        if let Some((o, c, widened)) = range {
            out.push(DayRange { date, open: minutes_hhmm(o), close: minutes_hhmm(c), widened });
        }
        day = day.succ_opt().expect("next day");
    }
    out
}

/// Available windows = each date's span minus the teacher's blocks.
/// Overlaps are computed on absolute instants (DST-safe) and then expressed
/// as studio wall-clock minutes; a block edge that falls mid-minute rounds
/// toward "blocked".
pub fn compute_windows(teacher_ids: &[i32], ranges: &[DayRange], blocks: &[BlockSpan]) -> Vec<Window> {
    use chrono::{TimeZone, Timelike};
    let mut by_user: HashMap<i32, Vec<&BlockSpan>> = HashMap::new();
    for b in blocks {
        by_user.entry(b.user_id).or_default().push(b);
    }
    let mut out = Vec::new();
    for r in ranges {
        let (Ok(date), Some(open), Some(close)) =
            (NaiveDate::parse_from_str(&r.date, "%Y-%m-%d"), hhmm_minutes(&r.open), hhmm_minutes(&r.close))
        else {
            continue;
        };
        let at = |m: i32| studio_local(date.and_time(NaiveTime::MIN) + chrono::Duration::minutes(m as i64)).timestamp();
        let (day_start, day_end) = (at(open), at(close));
        for &uid in teacher_ids {
            // Blocked minute ranges within [open, close].
            let mut cut: Vec<(i32, i32)> = Vec::new();
            for b in by_user.get(&uid).map(Vec::as_slice).unwrap_or(&[]) {
                if b.start >= day_end || b.end <= day_start {
                    continue;
                }
                let minute_of = |epoch: i64, round_up: bool| -> i32 {
                    let local = STUDIO_TZ.timestamp_opt(epoch, 0).single().map(|d| d.naive_local());
                    match local {
                        Some(l) if l.date() < date => 0,
                        Some(l) if l.date() > date => 24 * 60,
                        Some(l) => {
                            let m = (l.hour() * 60 + l.minute()) as i32;
                            if round_up && l.second() > 0 { m + 1 } else { m }
                        }
                        None => if round_up { 24 * 60 } else { 0 },
                    }
                };
                let from = if b.start <= day_start { open } else { minute_of(b.start, false).max(open) };
                let to = if b.end >= day_end { close } else { minute_of(b.end, true).min(close) };
                if to > from {
                    cut.push((from, to));
                }
            }
            cut.sort_unstable();
            let mut cursor = open;
            for (from, to) in cut {
                if from > cursor {
                    out.push(Window { sling_user_id: uid, date: r.date.clone(), start: minutes_hhmm(cursor), end: minutes_hhmm(from) });
                }
                cursor = cursor.max(to);
            }
            if cursor < close {
                out.push(Window { sling_user_id: uid, date: r.date.clone(), start: minutes_hhmm(cursor), end: minutes_hhmm(close) });
            }
        }
    }
    out.sort();
    out
}

/// Is `[start, end]` ("HH:MM") on `date` inside one of `uid`'s windows?
/// (The frontend's `slotInWindows` is the production twin; this one pins the
/// windows ⇔ blocks agreement in tests.)
#[cfg(test)]
pub fn slot_in_windows(windows: &[Window], uid: i32, date: &str, start: &str, end: &str) -> bool {
    windows
        .iter()
        .any(|w| w.sling_user_id == uid && w.date == date && w.start.as_str() <= start && end <= w.end.as_str())
}

/// Everything the Availability view needs for one month.
#[derive(Debug, Clone, Serialize)]
pub struct MonthAvailability {
    pub target_month: String,
    pub windows: Vec<Window>,
    pub day_ranges: Vec<DayRange>,
    /// false = studio hours were never set; spans come from class slots.
    pub hours_set: bool,
}

fn load_month_blocks(conn: &duckdb::Connection, first: NaiveDate, next_first: NaiveDate) -> Result<Vec<BlockSpan>, String> {
    // Compared as epoch seconds: the bundled DuckDB has no ICU (see
    // conflicts.rs), and epoch_us() is core.
    let lo = studio_local(first.and_time(NaiveTime::MIN)).timestamp();
    let hi = studio_local(next_first.and_time(NaiveTime::MIN)).timestamp();
    conn.prepare(
        "SELECT sling_user_id, epoch_us(starts_at) // 1000000, epoch_us(ends_at) // 1000000
         FROM availability_blocks
         WHERE epoch_us(starts_at) // 1000000 < ? AND epoch_us(ends_at) // 1000000 > ?",
    )
    .map_err(err)?
    .query_map(duckdb::params![hi, lo], |r| Ok(BlockSpan { user_id: r.get(0)?, start: r.get(1)?, end: r.get(2)? }))
    .map_err(err)?
    .collect::<Result<_, _>>()
    .map_err(err)
}

/// Compute one month's windows from the database (nothing is written).
pub fn compute_month(conn: &duckdb::Connection, target_month: &str) -> Result<MonthAvailability, String> {
    let (first, next) = month_bounds(target_month)?;
    let saved = load_studio_hours(conn)?;
    let hours_set = !saved.is_empty();
    let month_slots = load_slots(conn, target_month, target_month)?;
    let hours = if hours_set {
        saved
    } else {
        // Not set: hours are whatever the recent schedule implies.
        hours_from_slots(&load_slots(conn, &month_minus(target_month, 3)?, target_month)?)
    };
    let ranges = day_ranges(first, next, &hours, &month_slots);
    let teacher_ids: Vec<i32> = conn
        .prepare("SELECT sling_user_id FROM teachers WHERE active ORDER BY sling_user_id")
        .map_err(err)?
        .query_map([], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let blocks = load_month_blocks(conn, first, next)?;
    let windows = compute_windows(&teacher_ids, &ranges, &blocks);

    Ok(MonthAvailability {
        target_month: target_month.to_string(),
        windows,
        day_ranges: ranges,
        hours_set,
    })
}

fn stored_windows(conn: &duckdb::Connection, target_month: &str) -> Result<Vec<Window>, String> {
    let mut rows: Vec<Window> = conn
        .prepare(
            "SELECT sling_user_id, window_date, start_time, end_time
             FROM teacher_availability_windows WHERE target_month = ?",
        )
        .map_err(err)?
        .query_map(duckdb::params![target_month], |r| {
            Ok(Window { sling_user_id: r.get(0)?, date: r.get(1)?, start: r.get(2)?, end: r.get(3)? })
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    rows.sort();
    Ok(rows)
}

/// Recompute and store one month's windows (DELETE + INSERT for the month).
/// Compare-before-write: an unchanged month is not touched. Safe inside a
/// transaction or on a bare connection.
pub fn recompute_month(conn: &duckdb::Connection, target_month: &str) -> Result<MonthAvailability, String> {
    let month = compute_month(conn, target_month)?;
    if stored_windows(conn, target_month)? == month.windows {
        return Ok(month);
    }
    conn.execute("DELETE FROM teacher_availability_windows WHERE target_month = ?", duckdb::params![target_month])
        .map_err(err)?;
    let mut app = conn.appender("teacher_availability_windows").map_err(err)?;
    for w in &month.windows {
        app.append_row(duckdb::params![target_month, w.sling_user_id, w.date, w.start, w.end]).map_err(err)?;
    }
    app.flush().map_err(err)?;
    Ok(month)
}

/// Months that have data worth recomputing after a studio-hours change.
fn known_months(conn: &duckdb::Connection) -> Result<Vec<String>, String> {
    conn.prepare(
        "SELECT target_month FROM month_pulls
         UNION SELECT target_month FROM proposals
         UNION SELECT target_month FROM teacher_availability_windows
         ORDER BY 1",
    )
    .map_err(err)?
    .query_map([], |r| r.get(0))
    .map_err(err)?
    .collect::<Result<_, _>>()
    .map_err(err)
}

/// A compact per-teacher availability summary for the Claude editor: each
/// date's schedulable span, and per teacher only the dates on which they are
/// NOT available for the whole span (an empty list = unavailable all day).
pub fn editor_summary(month: &MonthAvailability, names: &HashMap<i32, String>) -> Value {
    let spans: BTreeMap<&str, String> =
        month.day_ranges.iter().map(|r| (r.date.as_str(), format!("{}-{}", r.open, r.close))).collect();
    let mut by_teacher: BTreeMap<i32, BTreeMap<&str, Vec<String>>> = BTreeMap::new();
    for uid in names.keys() {
        let entry = by_teacher.entry(*uid).or_default();
        for r in &month.day_ranges {
            entry.insert(r.date.as_str(), Vec::new());
        }
    }
    for w in &month.windows {
        if let Some(days) = by_teacher.get_mut(&w.sling_user_id) {
            days.entry(w.date.as_str()).or_default().push(format!("{}-{}", w.start, w.end));
        }
    }
    let teachers: Vec<Value> = by_teacher
        .into_iter()
        .map(|(uid, days)| {
            let exceptions: BTreeMap<&str, Vec<String>> = days
                .into_iter()
                .filter(|(date, wins)| !(wins.len() == 1 && spans.get(date) == Some(&wins[0])))
                .collect();
            serde_json::json!({
                "sling_user_id": uid,
                "name": names.get(&uid),
                "limited_days": exceptions,
            })
        })
        .collect();
    serde_json::json!({
        "studio_hours_by_date": spans,
        "teachers": teachers,
    })
}

// ============================================================
// Commands
// ============================================================

/// The month's computed availability (recomputed + stored on every read, so
/// it can't lag behind an edit that moved a class slot).
#[tauri::command(async)]
pub fn get_month_availability(db: State<'_, Db>, target_month: String) -> Result<MonthAvailability, String> {
    let conn = db.0.lock().map_err(err)?;
    recompute_month(&conn, &target_month)
}

#[derive(Debug, Clone, Serialize)]
pub struct StudioHoursDto {
    /// false = never saved; `days` is then what the schedule implies.
    pub set: bool,
    pub days: Vec<DayHours>,
}

#[tauri::command]
pub fn get_studio_hours(db: State<'_, Db>) -> Result<StudioHoursDto, String> {
    let conn = db.0.lock().map_err(err)?;
    let saved = load_studio_hours(&conn)?;
    if !saved.is_empty() {
        return Ok(StudioHoursDto { set: true, days: saved });
    }
    let current = crate::sling::studio_month_at(chrono::Utc::now());
    Ok(StudioHoursDto { set: false, days: suggest_studio_hours(&conn, &current)? })
}

/// Save the studio hours (an empty list clears them) and recompute every
/// known month's windows.
#[tauri::command(async)]
pub fn set_studio_hours(db: State<'_, Db>, days: Vec<DayHours>) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;
    save_studio_hours(&tx, &days)?;
    for month in known_months(&tx)? {
        recompute_month(&tx, &month)?;
    }
    tx.commit().map_err(err)?;
    let _ = conn.execute("CHECKPOINT", []);
    Ok(())
}

#[tauri::command]
pub fn suggest_studio_hours_from_schedule(db: State<'_, Db>) -> Result<Vec<DayHours>, String> {
    let conn = db.0.lock().map_err(err)?;
    let current = crate::sling::studio_month_at(chrono::Utc::now());
    suggest_studio_hours(&conn, &current)
}

#[cfg(test)]
mod tests;
