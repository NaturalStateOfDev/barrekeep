// Tests for block helpers, studio hours and windows. No network.

use super::*;
use serde_json::json;

const JULIA: i32 = 1930004;
const ALEX: i32 = 1930001;
const KAYLA: i32 = 1930002;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn db() -> duckdb::Connection {
    let conn = crate::db::open_in_memory().expect("open");
    crate::migrations::run(&conn).expect("migrations");
    conn.execute_batch(
        "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max) VALUES
           (1930001, 'Alex Braun', 4, 5), (1930002, 'Kayla Moore', 4, 5), (1930004, 'Julia Stone', 4, 5);
         INSERT INTO positions (sling_position_id, class_name) VALUES (101, 'Classic');",
    )
    .expect("seed");
    conn
}

// ---------- timestamps ----------
#[test]
fn parse_local_accepts_sling_timestamp_shapes() {
    let want = d("2026-11-03").and_hms_opt(9, 45, 0).unwrap();
    for s in [
        "2026-11-03T09:45:00-06:00",
        "2026-11-03T15:45:00Z",
        "2026-11-03T15:45:00.000Z",
        "2026-11-03T09:45:00-0600",
        "2026-11-03 09:45:00-06:00",
        "2026-11-03T09:45:00",
        "2026-11-03T09:45",
    ] {
        assert_eq!(parse_local(s), Some(want), "{s}");
    }
    assert_eq!(parse_local("2026-11-03"), Some(d("2026-11-03").and_time(NaiveTime::MIN)));
    assert_eq!(parse_local("soon"), None);
}

#[test]
fn leave_days_are_counted_within_the_month() {
    let n = |a: &str, b: &str| days_in_month(a, b, "2026-11");
    assert_eq!(n("2026-11-20T00:00:00-06:00", "2026-11-21T23:59:59-06:00"), 2);
    assert_eq!(n("2026-11-20T00:00:00-06:00", "2026-11-22T00:00:00-06:00"), 2, "exclusive midnight end");
    assert_eq!(n("2026-11-20T08:00:00-06:00", "2026-11-20T12:00:00-06:00"), 1);
    assert_eq!(n("2026-11-28T00:00:00-06:00", "2026-12-05T23:59:59-06:00"), 3, "clipped to the month");
    assert_eq!(n("2026-10-28T00:00:00-05:00", "2026-11-02T23:59:59-06:00"), 2);
    assert_eq!(n("nope", "2026-11-02T23:59:59-06:00"), 0);
}

// ---------- studio hours ----------

fn hours(open: &str, close: &str, closed_days: &[u8]) -> Vec<DayHours> {
    (0..7u8)
        .map(|wd| {
            if closed_days.contains(&wd) {
                DayHours { weekday: wd, closed: true, open: None, close: None }
            } else {
                DayHours { weekday: wd, closed: false, open: Some(open.into()), close: Some(close.into()) }
            }
        })
        .collect()
}

fn slot(date: &str, start: &str, end: &str) -> Slot {
    Slot { date: date.into(), start: start.into(), end: end.into() }
}

#[test]
fn studio_hours_round_trip_and_validation() {
    let conn = db();
    assert!(load_studio_hours(&conn).unwrap().is_empty(), "unset by default");
    let h = hours("05:30", "19:30", &[6]);
    save_studio_hours(&conn, &h).unwrap();
    assert_eq!(load_studio_hours(&conn).unwrap(), h);
    // Saving again replaces (no PK clash), and an empty list clears.
    save_studio_hours(&conn, &hours("06:00", "20:00", &[])).unwrap();
    assert_eq!(load_studio_hours(&conn).unwrap()[6].open.as_deref(), Some("06:00"));
    save_studio_hours(&conn, &[]).unwrap();
    assert!(load_studio_hours(&conn).unwrap().is_empty());

    let bad = |d: DayHours| save_studio_hours(&conn, &[d]).unwrap_err();
    assert!(bad(DayHours { weekday: 0, closed: false, open: Some("19:00".into()), close: Some("06:00".into()) })
        .contains("Monday"));
    assert!(bad(DayHours { weekday: 1, closed: false, open: None, close: Some("06:00".into()) }).contains("Tuesday"));
    assert!(bad(DayHours { weekday: 7, closed: true, open: None, close: None }).contains("weekday"));
    assert!(load_studio_hours(&conn).unwrap().is_empty(), "a rejected save changes nothing");
}

#[test]
fn hours_are_suggested_from_the_schedule() {
    // Mon Nov 2 / Tue Nov 3 2026.
    let slots = [
        slot("2026-11-02", "05:45", "06:45"),
        slot("2026-11-09", "17:30", "18:20"),
        slot("2026-11-03", "09:45", "10:45"),
        slot("2026-11-03", "bad", "10:45"),
    ];
    let h = hours_from_slots(&slots);
    assert_eq!(h.len(), 7);
    assert_eq!(h[0], DayHours { weekday: 0, closed: false, open: Some("05:45".into()), close: Some("18:20".into()) });
    assert_eq!((h[1].open.as_deref(), h[1].close.as_deref()), (Some("09:45"), Some("10:45")));
    assert!(h[2..].iter().all(|d| d.closed && d.open.is_none()));

    // From the database: recent Sling shifts + live drafts; dropped shifts
    // and archived drafts don't count.
    let conn = db();
    conn.execute_batch(
        "INSERT INTO external_sling_shifts (sling_shift_id, target_month, shift_date, start_time, end_time,
             sling_user_id, sling_position_id, status) VALUES
           (1, '2026-10', DATE '2026-10-05', '05:45', '06:45', 1930001, 101, 'published'),
           (2, '2026-10', DATE '2026-10-05', '18:30', '19:20', 1930001, 101, 'published'),
           (3, '2026-03', DATE '2026-03-02', '04:00', '05:00', 1930001, 101, 'published');
         INSERT INTO proposals (id, target_month, algorithm_version, parameters, is_current) VALUES
           (1, '2026-11', 'v9', '{}', TRUE), (2, '2026-11', 'v9', '{}', FALSE);
         INSERT INTO proposal_drafts (proposal_id, name, archived) VALUES (1, 'Draft 1', FALSE), (2, 'Old', TRUE);
         INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time, sling_position_id,
             sling_user_id, generation_reason, is_dropped) VALUES
           (1, DATE '2026-11-03', '09:45', '10:45', 101, 1930001, 'rotation', FALSE),
           (1, DATE '2026-11-03', '21:00', '22:00', 101, NULL, 'rotation', TRUE),
           (2, DATE '2026-11-03', '03:00', '04:00', 101, 1930001, 'rotation', FALSE);",
    )
    .unwrap();
    let s = suggest_studio_hours(&conn, "2026-11").unwrap();
    assert_eq!((s[0].open.as_deref(), s[0].close.as_deref()), (Some("05:45"), Some("19:20")), "March is too old");
    assert_eq!((s[1].open.as_deref(), s[1].close.as_deref()), (Some("09:45"), Some("10:45")));
    assert!(s[2].closed);
}

// ---------- windows ----------

fn span(uid: i32, from: &str, to: &str) -> BlockSpan {
    let at = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp();
    BlockSpan { user_id: uid, start: at(from), end: at(to) }
}

fn win(uid: i32, date: &str, start: &str, end: &str) -> Window {
    Window { sling_user_id: uid, date: date.into(), start: start.into(), end: end.into() }
}

#[test]
fn day_ranges_widen_for_off_hours_classes() {
    let (first, next) = (d("2026-11-02"), d("2026-11-09"));
    let h = hours("06:00", "19:00", &[6]); // closed Sundays
    let slots = [
        slot("2026-11-03", "09:45", "10:45"), // inside hours
        slot("2026-11-04", "05:00", "06:00"), // before opening
        slot("2026-11-05", "19:30", "20:30"), // after closing
        slot("2026-11-08", "09:00", "10:00"), // on a closed day
    ];
    let r = day_ranges(first, next, &h, &slots);
    let get = |date: &str| r.iter().find(|x| x.date == date).map(|x| (x.open.as_str(), x.close.as_str(), x.widened));
    assert_eq!(get("2026-11-02"), Some(("06:00", "19:00", false)));
    assert_eq!(get("2026-11-03"), Some(("06:00", "19:00", false)));
    assert_eq!(get("2026-11-04"), Some(("05:00", "19:00", true)));
    assert_eq!(get("2026-11-05"), Some(("06:00", "20:30", true)));
    assert_eq!(get("2026-11-07"), Some(("06:00", "19:00", false)));
    assert_eq!(get("2026-11-08"), Some(("09:00", "10:00", true)), "closed day opens for its class");
    // A closed day with no class has no span at all.
    assert_eq!(day_ranges(first, next, &h, &[]).len(), 6);
}

#[test]
fn windows_are_hours_minus_blocks() {
    let ranges = day_ranges(d("2026-11-03"), d("2026-11-05"), &hours("06:00", "19:00", &[]), &[]);
    let b = [
        span(JULIA, "2026-11-03T09:45:00-06:00", "2026-11-03T10:45:00-06:00"),
        span(JULIA, "2026-11-03T17:30:00-06:00", "2026-11-03T20:00:00-06:00"), // runs past closing
        span(JULIA, "2026-11-03T04:00:00-06:00", "2026-11-03T05:00:00-06:00"), // before opening: no effect
        span(KAYLA, "2026-11-02T00:00:00-06:00", "2026-11-05T00:00:00-06:00"), // multi-day leave
        span(ALEX, "2026-11-04T10:00:00-06:00", "2026-11-04T10:30:30-06:00"),  // ends mid-minute
        span(ALEX, "2026-11-04T10:15:00-06:00", "2026-11-04T11:00:00-06:00"),  // overlapping blocks merge
    ];
    let w = compute_windows(&[ALEX, KAYLA, JULIA], &ranges, &b);
    let of = |uid: i32| -> Vec<Window> { w.iter().filter(|x| x.sling_user_id == uid).cloned().collect() };
    assert_eq!(
        of(JULIA),
        [
            win(JULIA, "2026-11-03", "06:00", "09:45"),
            win(JULIA, "2026-11-03", "10:45", "17:30"),
            win(JULIA, "2026-11-04", "06:00", "19:00"),
        ]
    );
    assert!(of(KAYLA).is_empty(), "on leave both days");
    assert_eq!(
        of(ALEX),
        [
            win(ALEX, "2026-11-03", "06:00", "19:00"),
            win(ALEX, "2026-11-04", "06:00", "10:00"),
            win(ALEX, "2026-11-04", "11:00", "19:00"),
        ]
    );
    assert!(slot_in_windows(&w, JULIA, "2026-11-03", "10:45", "11:45"));
    assert!(!slot_in_windows(&w, JULIA, "2026-11-03", "09:45", "10:45"));
    assert!(!slot_in_windows(&w, JULIA, "2026-11-03", "09:00", "10:00"), "straddles a block edge");
}

/// November 2026 with Julia's recurring calendar blocks (Tuesday mornings,
/// every other Thursday evening, full-day Sundays through Nov 15, as the
/// calendar feed expands them per date), calendar blocks, a leave and a
/// month of class slots (including one outside studio hours).
fn november(conn: &duckdb::Connection) {
    conn.execute_batch(
        "INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at) VALUES
           (1930004, 'availability', TIMESTAMPTZ '2026-11-01 00:00:00-05', TIMESTAMPTZ '2026-11-02 00:00:00-06'),
           (1930004, 'availability', TIMESTAMPTZ '2026-11-03 09:45:00-06', TIMESTAMPTZ '2026-11-03 10:45:00-06'),
           (1930004, 'availability', TIMESTAMPTZ '2026-11-08 00:00:00-06', TIMESTAMPTZ '2026-11-09 00:00:00-06'),
           (1930004, 'availability', TIMESTAMPTZ '2026-11-10 09:45:00-06', TIMESTAMPTZ '2026-11-10 10:45:00-06'),
           (1930004, 'availability', TIMESTAMPTZ '2026-11-12 17:30:00-06', TIMESTAMPTZ '2026-11-12 18:30:00-06'),
           (1930004, 'availability', TIMESTAMPTZ '2026-11-15 00:00:00-06', TIMESTAMPTZ '2026-11-16 00:00:00-06'),
           (1930004, 'availability', TIMESTAMPTZ '2026-11-17 09:45:00-06', TIMESTAMPTZ '2026-11-17 10:45:00-06'),
           (1930004, 'availability', TIMESTAMPTZ '2026-11-24 09:45:00-06', TIMESTAMPTZ '2026-11-24 10:45:00-06'),
           (1930004, 'availability_set_pending', TIMESTAMPTZ '2026-11-26 17:30:00-06', TIMESTAMPTZ '2026-11-26 18:30:00-06'),
           (1930002, 'leave', TIMESTAMPTZ '2026-10-30 00:00:00-05', TIMESTAMPTZ '2026-11-03 23:59:59-06'),
           (1930001, 'availability', TIMESTAMPTZ '2026-11-04 08:45:00-06', TIMESTAMPTZ '2026-11-04 09:45:00-06'),
           (1930001, 'availability', TIMESTAMPTZ '2026-11-05 10:15:00-06', TIMESTAMPTZ '2026-11-05 12:00:00-06'),
           (1930001, 'availability', TIMESTAMPTZ '2026-11-06 20:30:00-06', TIMESTAMPTZ '2026-11-06 22:00:00-06');
         INSERT INTO proposals (id, target_month, algorithm_version, parameters, is_current)
           VALUES (1, '2026-11', 'v9', '{}', TRUE);",
    )
    .unwrap();
    let mut day = d("2026-11-01");
    while day < d("2026-12-01") {
        for (start, end) in [("05:45", "06:45"), ("09:45", "10:45"), ("17:30", "18:30")] {
            conn.execute(
                "INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time, sling_position_id,
                     sling_user_id, generation_reason) VALUES (1, CAST(? AS DATE), ?, ?, 101, NULL, 'rotation')",
                duckdb::params![day.to_string(), start, end],
            )
            .unwrap();
        }
        day = day.succ_opt().unwrap();
    }
    // A one-off evening workshop, outside any normal hours.
    conn.execute_batch(
        "INSERT INTO proposal_shifts (proposal_id, shift_date, start_time, end_time, sling_position_id,
             sling_user_id, generation_reason) VALUES (1, DATE '2026-11-06', '20:00', '21:00', 101, NULL, 'rotation');",
    )
    .unwrap();
}

#[test]
fn month_windows_with_and_without_studio_hours() {
    let conn = db();
    november(&conn);

    // Hours never set: the span is implied by the class slots.
    let implied = recompute_month(&conn, "2026-11").unwrap();
    assert!(!implied.hours_set);
    let range = |m: &MonthAvailability, date: &str| {
        m.day_ranges.iter().find(|r| r.date == date).map(|r| (r.open.clone(), r.close.clone(), r.widened))
    };
    assert_eq!(range(&implied, "2026-11-03"), Some(("05:45".into(), "18:30".into(), false)));
    assert_eq!(range(&implied, "2026-11-06"), Some(("05:45".into(), "21:00".into(), false)));
    assert_eq!(implied.day_ranges.len(), 30);

    // Hours set (closed Sundays): the off-hours workshop widens Nov 6 only,
    // and Sunday classes open the closed day for exactly their span.
    save_studio_hours(&conn, &hours("05:30", "19:30", &[6])).unwrap();
    let month = recompute_month(&conn, "2026-11").unwrap();
    assert!(month.hours_set);
    assert_eq!(range(&month, "2026-11-05"), Some(("05:30".into(), "19:30".into(), false)));
    assert_eq!(range(&month, "2026-11-06"), Some(("05:30".into(), "21:00".into(), true)));
    assert_eq!(range(&month, "2026-11-08"), Some(("05:45".into(), "18:30".into(), true)));

    let of = |uid: i32, date: &str| -> Vec<(String, String)> {
        month.windows.iter().filter(|w| w.sling_user_id == uid && w.date == date).map(|w| (w.start.clone(), w.end.clone())).collect()
    };
    let w = |a: &str, b: &str| (a.to_string(), b.to_string());
    // Julia: recurring Tuesday 9:45 block; full-day Sundays.
    assert_eq!(of(JULIA, "2026-11-03"), [w("05:30", "09:45"), w("10:45", "19:30")]);
    assert_eq!(of(JULIA, "2026-11-10"), [w("05:30", "09:45"), w("10:45", "19:30")]);
    assert!(of(JULIA, "2026-11-08").is_empty());
    assert_eq!(of(JULIA, "2026-11-22"), [w("05:45", "18:30")], "the Sunday set ended Nov 15");
    assert_eq!(of(JULIA, "2026-11-12"), [w("05:30", "17:30"), w("18:30", "19:30")]);
    // Kayla: leave that began in October covers Nov 1–3.
    assert!(of(KAYLA, "2026-11-02").is_empty());
    assert!(of(KAYLA, "2026-11-03").is_empty());
    assert_eq!(of(KAYLA, "2026-11-04"), [w("05:30", "19:30")]);
    // Alex: free until his evening block on the widened day.
    assert_eq!(of(ALEX, "2026-11-06"), [w("05:30", "20:30")]);

    // Stored, and replaced (not appended) on recompute.
    let stored = |c: &duckdb::Connection| -> i64 {
        c.query_row("SELECT count(*) FROM teacher_availability_windows WHERE target_month = '2026-11'", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(stored(&conn), month.windows.len() as i64);
    recompute_month(&conn, "2026-11").unwrap();
    assert_eq!(stored(&conn), month.windows.len() as i64);
    assert_eq!(stored_windows(&conn, "2026-11").unwrap(), month.windows);
    // Another month's rows are untouched by this month's rewrite.
    recompute_month(&conn, "2026-12").unwrap();
    save_studio_hours(&conn, &hours("05:30", "20:00", &[6])).unwrap();
    recompute_month(&conn, "2026-11").unwrap();
    let dec: i64 = conn
        .query_row("SELECT count(*) FROM teacher_availability_windows WHERE target_month = '2026-12'", [], |r| r.get(0))
        .unwrap();
    assert!(dec > 0);
    let monday = stored_windows(&conn, "2026-11").unwrap().into_iter().find(|w| w.date == "2026-11-02").unwrap();
    assert_eq!(monday.end, "20:00");
}

/// The windows are the blocks turned inside out, so the two can never
/// disagree: for every class slot and every teacher, the slot is inside one
/// of the teacher's windows ⇔ no block overlaps it. The block side uses the
/// exact test conflicts.rs applies (its own DST arithmetic), so this also
/// pins the chrono-tz and hand-rolled Central-time paths together.
#[test]
fn windows_and_blocks_agree_for_every_slot() {
    for hours_set in [false, true] {
        let conn = db();
        november(&conn);
        if hours_set {
            save_studio_hours(&conn, &hours("05:30", "19:30", &[6])).unwrap();
        }
        let month = recompute_month(&conn, "2026-11").unwrap();
        let slots = load_slots(&conn, "2026-11", "2026-11").unwrap();
        let block_rows: Vec<(i32, i64, i64)> = conn
            .prepare(
                "SELECT sling_user_id, epoch_us(starts_at) // 1000000, epoch_us(ends_at) // 1000000
                 FROM availability_blocks",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(slots.len() > 80 && block_rows.len() > 10);

        let (mut free, mut blocked) = (0, 0);
        for s in &slots {
            let start = crate::conflicts::local_to_epoch(&s.date, &s.start).unwrap();
            let end = crate::conflicts::local_to_epoch(&s.date, &s.end).unwrap();
            for uid in [ALEX, KAYLA, JULIA] {
                let has_block = block_rows.iter().any(|(u, bs, be)| *u == uid && *bs < end && *be > start);
                let in_window = slot_in_windows(&month.windows, uid, &s.date, &s.start, &s.end);
                assert_eq!(
                    in_window, !has_block,
                    "hours_set={hours_set} teacher {uid} {} {}–{}: window says {in_window}, blocks say {}",
                    s.date, s.start, s.end, !has_block
                );
                if has_block { blocked += 1 } else { free += 1 }
            }
        }
        assert!(free > 200 && blocked >= 15, "free={free} blocked={blocked}");
    }
}

#[test]
fn editor_summary_lists_only_limited_days() {
    let conn = db();
    november(&conn);
    save_studio_hours(&conn, &hours("05:30", "19:30", &[6])).unwrap();
    let month = compute_month(&conn, "2026-11").unwrap();
    let names: HashMap<i32, String> =
        [(ALEX, "Alex Braun".to_string()), (JULIA, "Julia Stone".to_string())].into_iter().collect();
    let v = editor_summary(&month, &names);
    assert_eq!(v["studio_hours_by_date"]["2026-11-03"], "05:30-19:30");
    assert_eq!(v["studio_hours_by_date"]["2026-11-06"], "05:30-21:00");
    let teacher = |uid: i32| v["teachers"].as_array().unwrap().iter().find(|t| t["sling_user_id"] == uid).unwrap().clone();
    let julia = teacher(JULIA);
    assert_eq!(julia["name"], "Julia Stone");
    assert_eq!(julia["limited_days"]["2026-11-03"], json!(["05:30-09:45", "10:45-19:30"]));
    assert_eq!(julia["limited_days"]["2026-11-08"], json!([]), "unavailable all day");
    assert!(julia["limited_days"].get("2026-11-04").is_none(), "fully available days are omitted");
    // Alex is limited on three days only.
    assert_eq!(teacher(ALEX)["limited_days"].as_object().unwrap().len(), 3);
    assert_eq!(v["teachers"].as_array().unwrap().len(), 2, "only the teachers asked for");
}

#[test]
fn propose_sees_every_source_as_blocked() {
    assert_eq!(propose_event_type("leave"), "leave");
    assert_eq!(propose_event_type("availability"), "availability");
    // Legacy set-derived rows (no longer written) and unknown sources.
    assert_eq!(propose_event_type("availability_set"), "availability");
    assert_eq!(propose_event_type(SOURCE_SET_PENDING), "availability");
    assert_eq!(propose_event_type("something_new"), "availability");
}
