// Teacher availability beyond the one-off calendar blocks:
//
//   1. Recurring availability SETS (Sling `GET /availability?userId=…`).
//      Teachers enter their standing unavailability as a set: a list of
//      (weekday, time range) entries that repeats every N weeks from `start`
//      until `until`. The calendar feed does not reliably carry these, so
//      they are pulled per teacher, stored raw (`sling_availability_sets`)
//      and expanded here into concrete `availability_blocks` rows for a month
//      (source 'availability_set' / 'availability_set_pending').
//   2. Studio hours (`studio_hours`) — per-weekday open/close.
//   3. Computed AVAILABLE windows (`teacher_availability_windows`): each
//      date's studio hours, widened to any class slot outside them, minus
//      every block. Blocks stay the source of truth (propose.py and
//      conflicts.rs decide from blocks); the windows are the same facts
//      turned inside out for the Availability view, the day editor and the
//      Claude editor. `windows_and_blocks_agree_for_every_slot` pins the two
//      together.
//
// Sling's naming is backward: an "availability" entry is time the teacher is
// UNAVAILABLE (CLAUDE.md). Every source in availability_blocks is blocked
// time.
//
// The `interval` wire format of a set is not documented beyond "recurrence
// interval i.e. every week, every two weeks", so `parse_interval` accepts
// every plausible spelling and anything else is kept as an UNINTERPRETED set:
// no blocks, a stored `problem`, and a visible warning — never a silent drop.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::db::Db;
use crate::sling::{studio_iso, studio_local, STUDIO_TZ};

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

// ============================================================
// Block sources
// ============================================================

// `availability` — one-off blocked time from the calendar feed (the Sling
// event type, stored verbatim by commands::write_month_events).
/// Approved time off from the calendar feed (Sling type `leave`).
pub const SOURCE_LEAVE: &str = "leave";
/// An occurrence of an approved recurring availability set.
pub const SOURCE_SET: &str = "availability_set";
/// An occurrence of a set that has no `approved` timestamp yet. Still
/// blocked — scheduling over a pending request is the worse mistake.
pub const SOURCE_SET_PENDING: &str = "availability_set_pending";

/// The event `type` propose.py sees for a block source. It only knows
/// 'leave' and 'availability' (both blocked); the set-derived sources are
/// presented as 'availability' so every script version — the baseline and
/// any adopted copy — treats them as blocked without changing a line.
pub fn propose_event_type(source: &str) -> &'static str {
    if source == SOURCE_LEAVE {
        "leave"
    } else {
        "availability"
    }
}

// ============================================================
// Parsing availability sets
// ============================================================

/// One (weekday, time range) entry of a set, as Sling sent it.
#[derive(Debug, Clone, PartialEq)]
pub struct SetEntry {
    pub dtstart: String,
    pub dtend: String,
    pub full_day: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedSet {
    /// Sling's set id as text (may be absent).
    pub set_id: Option<String>,
    /// The owning user when the set names one.
    pub user_id: Option<i64>,
    pub name: Option<String>,
    pub start: Option<String>,
    pub until: Option<String>,
    /// `interval` exactly as sent, as JSON text (`"P1W"`, `2`, `null`).
    pub interval_raw: Option<String>,
    /// Interpreted recurrence step in days; None = not understood.
    pub interval_days: Option<u32>,
    pub approved: Option<String>,
    pub entries: Vec<SetEntry>,
    /// Why the set can't be (fully) expanded, if anything is wrong.
    pub problem: Option<String>,
}

impl ParsedSet {
    pub fn pending(&self) -> bool {
        self.approved.is_none()
    }
}

fn json_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn json_i64(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn looks_like_set(o: &serde_json::Map<String, Value>) -> bool {
    o.get("availabilities").is_some_and(Value::is_array)
        && ["interval", "start", "until", "user", "approved", "name"].iter().any(|k| o.contains_key(*k))
}

/// The list of raw sets inside an `/availability` response: a bare array
/// (the documented shape), a single set object, or an array wrapped in an
/// object under a conventional key. None = unrecognized shape.
pub fn unwrap_sets(doc: &Value) -> Option<Vec<Value>> {
    match doc {
        Value::Array(a) => Some(a.clone()),
        Value::Object(o) => {
            if looks_like_set(o) {
                return Some(vec![doc.clone()]);
            }
            for key in ["availabilitySets", "availability_sets", "sets", "availability", "data", "results", "items", "rules"] {
                match o.get(key) {
                    Some(Value::Array(a)) => return Some(a.clone()),
                    Some(inner @ Value::Object(_)) => {
                        if let Some(found) = unwrap_sets(inner) {
                            return Some(found);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        _ => None,
    }
}

const NUMBER_WORDS: [(&str, u32); 6] =
    [("one", 1), ("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6)];

fn count_word(s: &str) -> Option<u32> {
    s.parse::<u32>().ok().or_else(|| NUMBER_WORDS.iter().find(|(w, _)| *w == s).map(|(_, n)| *n))
}

/// A bare number: 1–6 = that many weeks; a multiple of 604,800 = seconds.
/// Anything else (7? 14? weeks or days?) is ambiguous and not guessed.
fn interval_from_number(n: i64) -> Option<u32> {
    const WEEK_SECS: i64 = 604_800;
    if (1..=6).contains(&n) {
        Some(n as u32 * 7)
    } else if n >= WEEK_SECS && n % WEEK_SECS == 0 && n / WEEK_SECS <= 52 {
        Some((n / WEEK_SECS) as u32 * 7)
    } else {
        None
    }
}

/// "<n> week(s)" / "<n> day(s)" → days.
fn interval_from_unit_phrase(s: &str) -> Option<u32> {
    let mut parts = s.split_whitespace();
    let (first, second, third) = (parts.next()?, parts.next(), parts.next());
    if third.is_some() {
        return None;
    }
    let (count, unit) = match second {
        Some(unit) => (count_word(first)?, unit),
        None => (1, first),
    };
    if count == 0 {
        return None;
    }
    match unit {
        "week" | "weeks" | "wk" | "wks" | "w" => Some(count * 7),
        "day" | "days" | "d" => Some(count),
        _ => None,
    }
}

/// Interpret a set's `interval` as a recurrence step in days (7 = weekly,
/// 14 = every two weeks). Accepted spellings:
///   - numbers / numeric strings: 1–6 = weeks; multiples of 604800 = seconds
///   - ISO-8601 durations: `P1W`, `P2W`, `P7D`, `P14D`
///   - RRULE fragments: `FREQ=WEEKLY`, `FREQ=WEEKLY;INTERVAL=2`,
///     `RRULE:FREQ=DAILY;INTERVAL=7`
///   - words: `weekly`, `every week`, `biweekly`, `fortnightly`, `every other
///     week`, `every two weeks`, `every 2 weeks`, `2 weeks`, `daily`
///   - Python timedelta text: `7 days, 0:00:00`, `14 days`
///
/// None for anything else (monthly, yearly, null, free text): the caller
/// records the set as uninterpreted instead of guessing.
pub fn parse_interval(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => {
            let i = n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))?;
            interval_from_number(i)
        }
        Value::String(raw) => {
            let s = raw.trim().to_ascii_lowercase();
            if s.is_empty() {
                return None;
            }
            if let Ok(n) = s.parse::<i64>() {
                return interval_from_number(n);
            }
            // RRULE
            if s.contains("freq=") {
                let body = s.strip_prefix("rrule:").unwrap_or(&s);
                let mut freq = None;
                let mut every = 1u32;
                for part in body.split(';') {
                    match part.split_once('=') {
                        Some(("freq", f)) => freq = Some(f.trim().to_string()),
                        Some(("interval", n)) => every = n.trim().parse().ok().filter(|n| *n > 0)?,
                        _ => {}
                    }
                }
                return match freq.as_deref() {
                    Some("weekly") => Some(every * 7),
                    Some("daily") => Some(every),
                    _ => None,
                };
            }
            // ISO-8601 duration: PnW / PnD.
            if let Some(body) = s.strip_prefix('p') {
                if let Some(n) = body.strip_suffix('w').and_then(|n| n.parse::<u32>().ok()) {
                    return (n > 0).then_some(n * 7);
                }
                if let Some(n) = body.strip_suffix('d').and_then(|n| n.parse::<u32>().ok()) {
                    return (n > 0).then_some(n);
                }
                // fall through: "per week" etc. are not expected, but harmless
            }
            // Python timedelta text: "7 days, 0:00:00".
            let s = match s.split_once(',') {
                Some((head, tail)) if tail.trim().chars().all(|c| c == '0' || c == ':') => head.trim().to_string(),
                _ => s,
            };
            let s = s.replace(['-', '_'], " ");
            let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
            let core = s.strip_prefix("every ").or_else(|| s.strip_prefix("each ")).unwrap_or(&s);
            match core {
                "weekly" => Some(7),
                "daily" => Some(1),
                "biweekly" | "bi weekly" | "fortnight" | "fortnightly" | "other week" | "second week"
                | "2nd week" => Some(14),
                "other day" | "second day" | "2nd day" => Some(2),
                other => interval_from_unit_phrase(other),
            }
        }
        _ => None,
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

/// Parse one raw set defensively: ids may be strings or numbers, fields may
/// be missing. Never fails — whatever is wrong ends up in `problem`.
pub fn parse_set(raw: &Value) -> ParsedSet {
    let text = |key: &str| raw.get(key).and_then(json_text);
    let user_id = match raw.get("user") {
        Some(Value::Object(u)) => u.get("id").and_then(json_i64),
        Some(v) => json_i64(v),
        None => None,
    }
    .or_else(|| raw.get("userId").and_then(json_i64));
    let interval_value = raw.get("interval").filter(|v| !v.is_null()).or_else(|| raw.get("rrule"));
    let interval_raw = interval_value.map(|v| v.to_string());
    let interval_days = interval_value.and_then(parse_interval);
    let approved = match raw.get("approved") {
        Some(Value::Bool(true)) => Some("true".to_string()),
        Some(v) => json_text(v),
        None => None,
    };
    let mut bad_entries = 0usize;
    let entries: Vec<SetEntry> = raw
        .get("availabilities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|it| {
                    let get = |k: &str| it.get(k).and_then(|v| v.as_str()).map(str::to_string);
                    let full_day = ["fullDay", "full_day", "fullday"]
                        .iter()
                        .any(|k| it.get(*k).and_then(Value::as_bool).unwrap_or(false));
                    match (get("dtstart"), get("dtend")) {
                        (Some(dtstart), Some(dtend)) => Some(SetEntry { dtstart, dtend, full_day }),
                        // A full-day entry needs only its start date.
                        (Some(dtstart), None) if full_day => {
                            Some(SetEntry { dtend: dtstart.clone(), dtstart, full_day })
                        }
                        _ => {
                            bad_entries += 1;
                            None
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let start = text("start");
    let until = text("until");
    let mut problems: Vec<String> = Vec::new();
    if interval_days.is_none() {
        problems.push(format!(
            "interval {} not understood",
            interval_raw.as_deref().unwrap_or("missing")
        ));
    }
    for (label, value) in [("start", &start), ("until", &until)] {
        if let Some(v) = value {
            if parse_local(v).is_none() {
                problems.push(format!("{label} \"{v}\" is not a date"));
            }
        }
    }
    if bad_entries > 0 {
        problems.push(format!("{bad_entries} entr{} without dtstart/dtend", if bad_entries == 1 { "y" } else { "ies" }));
    }
    let unreadable = entries
        .iter()
        .filter(|e| entry_span(e).is_none())
        .count();
    if unreadable > 0 {
        problems.push(format!("{unreadable} entr{} with unreadable times", if unreadable == 1 { "y" } else { "ies" }));
    }

    ParsedSet {
        set_id: raw.get("id").and_then(json_text),
        user_id,
        name: text("name"),
        start,
        until,
        interval_raw,
        interval_days,
        approved,
        entries,
        problem: (!problems.is_empty()).then(|| problems.join("; ")),
    }
}

/// The first occurrence of an entry as a studio-local (start, end). A
/// full-day entry covers whole local days, midnight to midnight.
fn entry_span(e: &SetEntry) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let start = parse_local(&e.dtstart)?;
    let end = parse_local(&e.dtend)?;
    if e.full_day {
        let first = start.date();
        // dtend may be the same instant, 23:59:59 that day, or the next
        // midnight (exclusive) — all mean "through `first`".
        let last = if end > start { (end - chrono::Duration::seconds(1)).date() } else { first };
        let last = last.max(first);
        Some((first.and_time(NaiveTime::MIN), last.succ_opt()?.and_time(NaiveTime::MIN)))
    } else if end > start {
        Some((start, end))
    } else {
        None
    }
}

/// Every occurrence of `set` that STARTS on a date in `[first, next_first)`,
/// as studio-local wall-clock (start, end) pairs.
///
/// Each entry repeats every `interval_days` from its OWN dtstart — so an
/// every-two-weeks set keeps the parity of the week its entry sits in, and a
/// set spanning both weeks of its cycle keeps both. Wall-clock times are
/// preserved across DST changes (9:45 stays 9:45). Occurrences before the
/// set's `start` date or after its `until` date (inclusive — the safer
/// reading) are left out. An uninterpreted set yields nothing.
pub fn expand_set(set: &ParsedSet, first: NaiveDate, next_first: NaiveDate) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    let Some(step) = set.interval_days.filter(|d| *d > 0).map(i64::from) else {
        return Vec::new();
    };
    let bound = |v: &Option<String>| -> Result<Option<NaiveDate>, ()> {
        match v {
            None => Ok(None),
            Some(s) => parse_local(s).map(|dt| Some(dt.date())).ok_or(()),
        }
    };
    let (Ok(from), Ok(until)) = (bound(&set.start), bound(&set.until)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in &set.entries {
        let Some((start, end)) = entry_span(entry) else { continue };
        let length = end - start;
        let anchor = start.date();
        // Smallest k >= 0 with anchor + k*step >= first.
        let gap = (first - anchor).num_days();
        let mut k = if gap <= 0 { 0 } else { (gap + step - 1) / step };
        loop {
            let Some(day) = anchor.checked_add_signed(chrono::Duration::days(k * step)) else { break };
            if day >= next_first {
                break;
            }
            k += 1;
            if from.is_some_and(|f| day < f) || until.is_some_and(|u| day > u) {
                continue;
            }
            let occ_start = day.and_time(start.time());
            out.push((occ_start, occ_start + length));
        }
    }
    out.sort();
    out.dedup();
    out
}

// ============================================================
// Storing sets + writing their blocks
// ============================================================

/// Replace one teacher's stored sets with what Sling just returned (DELETE +
/// INSERT, never UPDATE). Sets that name a different user are ignored — they
/// arrive with their owner's request. Must run inside a transaction.
pub fn replace_sets_for_user(conn: &duckdb::Connection, user_id: i64, raw_sets: &[Value]) -> Result<usize, String> {
    conn.execute("DELETE FROM sling_availability_sets WHERE sling_user_id = ?", duckdb::params![user_id as i32])
        .map_err(err)?;
    let mut stored = 0usize;
    for raw in raw_sets {
        let set = parse_set(raw);
        if set.user_id.is_some_and(|u| u != user_id) {
            continue;
        }
        conn.execute(
            "INSERT INTO sling_availability_sets
                (sling_set_id, sling_user_id, name, starts_on, until, interval_raw, interval_days,
                 approved_at, pending, availability_count, problem, raw_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            duckdb::params![
                set.set_id,
                user_id as i32,
                set.name,
                set.start,
                set.until,
                set.interval_raw,
                set.interval_days.map(|d| d as i32),
                set.approved,
                set.pending(),
                set.entries.len() as i32,
                set.problem,
                raw.to_string(),
            ],
        )
        .map_err(err)?;
        stored += 1;
    }
    Ok(stored)
}

/// Stored sets, re-parsed from their raw JSON (one code path for parsing).
pub fn load_sets(conn: &duckdb::Connection) -> Result<Vec<(i32, ParsedSet)>, String> {
    let rows: Vec<(i32, String)> = conn
        .prepare("SELECT sling_user_id, raw_json FROM sling_availability_sets ORDER BY sling_user_id, id")
        .map_err(err)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    Ok(rows
        .into_iter()
        .filter_map(|(uid, raw)| serde_json::from_str::<Value>(&raw).ok().map(|v| (uid, parse_set(&v))))
        .collect())
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

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct SetBlockStats {
    /// Blocks written from approved sets.
    pub written: i64,
    /// Blocks written from sets pending approval.
    pub pending: i64,
    /// Occurrences skipped because the calendar already supplied the
    /// identical block (same teacher, start and end).
    pub duplicates: i64,
}

/// Expand the stored sets into `availability_blocks` for one month. Call
/// AFTER the month's calendar blocks are written (and its old blocks
/// deleted): occurrences identical to an existing block are skipped. Only
/// roster teachers get blocks (availability_blocks has an FK into teachers).
/// Must run inside a transaction.
pub fn write_set_blocks(
    conn: &duckdb::Connection,
    target_month: &str,
    roster_ids: &HashSet<i32>,
) -> Result<SetBlockStats, String> {
    let (first, next) = month_bounds(target_month)?;
    let (m_start, m_end) = crate::sling::month_range(target_month).map_err(err)?;
    let mut seen: HashSet<(i32, i64, i64)> = conn
        .prepare(
            "SELECT sling_user_id, epoch_us(starts_at) // 1000000, epoch_us(ends_at) // 1000000
             FROM availability_blocks
             WHERE starts_at >= CAST(? AS TIMESTAMPTZ) AND starts_at <= CAST(? AS TIMESTAMPTZ)",
        )
        .map_err(err)?
        .query_map(duckdb::params![&m_start, &m_end], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;

    let mut stats = SetBlockStats::default();
    // Approved sets first, so an approved and a pending set describing the
    // same block leave the approved label.
    let mut sets = load_sets(conn)?;
    sets.sort_by_key(|(_, s)| s.pending());
    for (uid, set) in &sets {
        if !roster_ids.contains(uid) {
            continue;
        }
        let source = if set.pending() { SOURCE_SET_PENDING } else { SOURCE_SET };
        for (start, end) in expand_set(set, first, next) {
            let (s, e) = (studio_local(start), studio_local(end));
            if !seen.insert((*uid, s.timestamp(), e.timestamp())) {
                stats.duplicates += 1;
                continue;
            }
            conn.execute(
                "INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at)
                 VALUES (?, ?, CAST(? AS TIMESTAMPTZ), CAST(? AS TIMESTAMPTZ))",
                duckdb::params![uid, source, studio_iso(start), studio_iso(end)],
            )
            .map_err(err)?;
            if set.pending() {
                stats.pending += 1;
            } else {
                stats.written += 1;
            }
        }
    }
    Ok(stats)
}

/// A stored set the app could not (fully) interpret.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SetIssue {
    pub sling_user_id: i32,
    pub teacher_name: Option<String>,
    pub name: Option<String>,
    pub interval_raw: Option<String>,
    pub problem: String,
}

pub fn set_issues(conn: &duckdb::Connection) -> Result<Vec<SetIssue>, String> {
    conn.prepare(
        "SELECT s.sling_user_id, t.display_name, s.name, s.interval_raw, s.problem
         FROM sling_availability_sets s
         LEFT JOIN teachers t ON t.sling_user_id = s.sling_user_id
         WHERE s.problem IS NOT NULL
         ORDER BY t.display_name, s.id",
    )
    .map_err(err)?
    .query_map([], |r| {
        Ok(SetIssue {
            sling_user_id: r.get(0)?,
            teacher_name: r.get(1)?,
            name: r.get(2)?,
            interval_raw: r.get(3)?,
            problem: r.get(4)?,
        })
    })
    .map_err(err)?
    .collect::<Result<_, _>>()
    .map_err(err)
}

/// The warning shown wherever availability is pulled or viewed.
pub fn uninterpreted_warning(n: usize) -> Option<String> {
    (n > 0).then(|| {
        format!(
            "{n} availability set{} from Sling couldn't be interpreted — schedule may miss unavailability; see raw pull file",
            if n == 1 { "" } else { "s" }
        )
    })
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
    pub set_count: i64,
    pub pending_set_count: i64,
    pub set_issues: Vec<SetIssue>,
    pub warnings: Vec<String>,
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

    let (set_count, pending_set_count): (i64, i64) = conn
        .query_row(
            "SELECT count(*), coalesce(sum(CASE WHEN pending THEN 1 ELSE 0 END), 0) FROM sling_availability_sets",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    let issues = set_issues(conn)?;
    let warnings = uninterpreted_warning(issues.len()).into_iter().collect();
    Ok(MonthAvailability {
        target_month: target_month.to_string(),
        windows,
        day_ranges: ranges,
        hours_set,
        set_count,
        pending_set_count,
        set_issues: issues,
        warnings,
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
