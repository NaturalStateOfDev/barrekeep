// Fixture-driven tests for availability sets, studio hours and windows.
// No network: the fixture (test_fixtures/sling_availability_sets.json) is
// built from the shapes in Sling's Swagger spec.

use super::*;
use serde_json::json;

const JULIA: i32 = 1930004;
const ALEX: i32 = 1930001;
const KAYLA: i32 = 1930002;

fn fixture() -> Value {
    let raw = std::fs::read_to_string("test_fixtures/sling_availability_sets.json").expect("fixture present");
    serde_json::from_str(&raw).expect("fixture is JSON")
}

fn julia_sets() -> Vec<Value> {
    unwrap_sets(&fixture()["user_julia"]).expect("array")
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn month(m: &str) -> (NaiveDate, NaiveDate) {
    month_bounds(m).unwrap()
}

/// Occurrences as "YYYY-MM-DD HH:MM→HH:MM" (or "…→MM-DD HH:MM" across days).
fn occurrences(set: &ParsedSet, m: &str) -> Vec<String> {
    let (first, next) = month(m);
    expand_set(set, first, next)
        .into_iter()
        .map(|(s, e)| {
            if s.date() == e.date() {
                format!("{}→{}", s.format("%Y-%m-%d %H:%M"), e.format("%H:%M"))
            } else {
                format!("{}→{}", s.format("%Y-%m-%d %H:%M"), e.format("%m-%d %H:%M"))
            }
        })
        .collect()
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

fn roster() -> HashSet<i32> {
    [ALEX, KAYLA, JULIA].into_iter().collect()
}

/// (user, source, starts_at UTC, ends_at UTC) for every stored block.
fn blocks(conn: &duckdb::Connection) -> Vec<(i32, String, String, String)> {
    conn.prepare(concat!(
        "SELECT sling_user_id, source, ",
        crate::db::utc_iso!("starts_at"),
        ", ",
        crate::db::utc_iso!("ends_at"),
        " FROM availability_blocks ORDER BY starts_at, sling_user_id, source"
    ))
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

// ---------- parsing ----------

#[test]
fn unwraps_every_response_shape() {
    let fx = fixture();
    // Documented shape: a bare array.
    assert_eq!(unwrap_sets(&fx["user_julia"]).unwrap().len(), 4);
    // Wrapped under a conventional key.
    assert_eq!(unwrap_sets(&fx["user_alex_wrapped"]).unwrap().len(), 1);
    assert_eq!(unwrap_sets(&json!({"availabilitySets": [{"interval": 1}]})).unwrap().len(), 1);
    assert_eq!(unwrap_sets(&json!({"data": {"sets": [{}, {}]}})).unwrap().len(), 2);
    // A teacher with no sets.
    assert!(unwrap_sets(&fx["user_kayla_none"]).unwrap().is_empty());
    // A single set object (GET /availability/{id}).
    let single = fx["user_julia"][0].clone();
    assert_eq!(unwrap_sets(&single).unwrap(), vec![single]);
    // Unrecognized shapes are reported, not treated as "no sets".
    assert!(unwrap_sets(&json!({"error": "nope"})).is_none());
    assert!(unwrap_sets(&json!("oops")).is_none());
}

#[test]
fn parses_ids_as_strings_or_numbers() {
    let sets: Vec<ParsedSet> = julia_sets().iter().map(parse_set).collect();
    // Numeric set id + numeric user id.
    assert_eq!(sets[0].set_id.as_deref(), Some("9001"));
    assert_eq!(sets[0].user_id, Some(1930004));
    // String set id + string user id.
    assert_eq!(sets[1].set_id.as_deref(), Some("9002"));
    assert_eq!(sets[1].user_id, Some(1930004));
    assert_eq!(sets[0].name.as_deref(), Some("Fall mornings"));
    assert_eq!(sets[0].interval_raw.as_deref(), Some("\"P1W\""));
    assert_eq!(sets[1].interval_raw.as_deref(), Some("2"));
    assert_eq!(sets[0].entries.len(), 1);
    assert!(!sets[0].entries[0].full_day);
    assert!(sets[2].entries[0].full_day);
    // A flat userId and a missing id are fine too.
    let flat = parse_set(&json!({"userId": "42", "interval": 1, "availabilities": []}));
    assert_eq!((flat.user_id, flat.set_id), (Some(42), None));
    assert!(flat.problem.is_none());
}

#[test]
fn approved_null_or_missing_is_pending() {
    let sets: Vec<ParsedSet> = julia_sets().iter().map(parse_set).collect();
    assert!(!sets[0].pending());
    assert!(sets[2].pending(), "approved: null");
    assert!(parse_set(&json!({"interval": 1})).pending(), "approved missing");
    assert!(parse_set(&json!({"interval": 1, "approved": ""})).pending());
    assert!(parse_set(&json!({"interval": 1, "approved": false})).pending());
    assert!(!parse_set(&json!({"interval": 1, "approved": true})).pending());
}

#[test]
fn interval_forms() {
    let days = |v: Value| parse_interval(&v);
    // Integer weeks, as numbers and numeric strings.
    assert_eq!(days(json!(1)), Some(7));
    assert_eq!(days(json!(2)), Some(14));
    assert_eq!(days(json!("1")), Some(7));
    assert_eq!(days(json!("2")), Some(14));
    assert_eq!(days(json!(2.0)), Some(14));
    // Seconds.
    assert_eq!(days(json!(604800)), Some(7));
    assert_eq!(days(json!("1209600")), Some(14));
    // ISO-8601 durations.
    assert_eq!(days(json!("P1W")), Some(7));
    assert_eq!(days(json!("P2W")), Some(14));
    assert_eq!(days(json!("p7d")), Some(7));
    assert_eq!(days(json!("P14D")), Some(14));
    // Words.
    for (text, want) in [
        ("weekly", 7),
        ("Weekly", 7),
        ("week", 7),
        ("every week", 7),
        ("Every Week", 7),
        ("1 week", 7),
        ("every 1 week", 7),
        ("biweekly", 14),
        ("bi-weekly", 14),
        ("fortnightly", 14),
        ("every other week", 14),
        ("every two weeks", 14),
        ("every 2 weeks", 14),
        ("2 weeks", 14),
        ("every three weeks", 21),
        ("daily", 1),
        ("every day", 1),
        // Python timedelta text.
        ("7 days, 0:00:00", 7),
        ("14 days", 14),
    ] {
        assert_eq!(days(json!(text)), Some(want), "{text}");
    }
    // RRULE-like.
    assert_eq!(days(json!("FREQ=WEEKLY")), Some(7));
    assert_eq!(days(json!("FREQ=WEEKLY;INTERVAL=2")), Some(14));
    assert_eq!(days(json!("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=TU")), Some(14));
    assert_eq!(days(json!("FREQ=DAILY;INTERVAL=7")), Some(7));
    // Not understood — never guessed.
    for bad in [
        json!(null),
        json!(""),
        json!(0),
        json!(7), // 7 weeks or 7 days?
        json!(-1),
        json!("monthly"),
        json!("every month"),
        json!("FREQ=MONTHLY"),
        json!("FREQ=WEEKLY;INTERVAL=0"),
        json!("P1M"),
        json!("whenever"),
        json!(true),
        json!({"weeks": 1}),
    ] {
        assert_eq!(days(bad.clone()), None, "{bad}");
    }
}

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

// ---------- expansion ----------

#[test]
fn weekly_set_recurs_forever_on_its_weekday() {
    let set = parse_set(&julia_sets()[0]);
    assert_eq!(
        occurrences(&set, "2026-11"),
        ["2026-11-03 09:45→10:45", "2026-11-10 09:45→10:45", "2026-11-17 09:45→10:45", "2026-11-24 09:45→10:45"]
    );
    // until: null = forever.
    assert_eq!(occurrences(&set, "2031-06").len(), 4);
    // Nothing before the set starts.
    assert!(occurrences(&set, "2026-08").is_empty());
    // The first occurrence is the entry itself.
    assert_eq!(occurrences(&set, "2026-09")[0], "2026-09-01 09:45→10:45");
}

#[test]
fn until_is_inclusive_and_start_is_respected() {
    let set = parse_set(&json!({
        "interval": "P1W",
        "start": "2026-11-10T00:00:00-06:00",
        "until": "2026-11-17T00:00:00-06:00",
        "availabilities": [{"dtstart": "2026-10-06T09:45:00-05:00", "dtend": "2026-10-06T10:45:00-05:00"}]
    }));
    // The entry sits before `start`: occurrences only from `start` on, and
    // the `until` date itself still counts (blocked is the safer reading).
    assert_eq!(occurrences(&set, "2026-11"), ["2026-11-10 09:45→10:45", "2026-11-17 09:45→10:45"]);
    assert!(occurrences(&set, "2026-10").is_empty());
    assert!(occurrences(&set, "2026-12").is_empty());
}

#[test]
fn biweekly_keeps_the_parity_of_its_start() {
    let set = parse_set(&julia_sets()[1]);
    // Anchored on Thu Sep 3: Sep 3, 17 · Oct 1, 15, 29 · Nov 12, 26.
    assert_eq!(occurrences(&set, "2026-09"), ["2026-09-03 17:30→18:30", "2026-09-17 17:30→18:30"]);
    assert_eq!(
        occurrences(&set, "2026-10"),
        ["2026-10-01 17:30→18:30", "2026-10-15 17:30→18:30", "2026-10-29 17:30→18:30"]
    );
    assert_eq!(occurrences(&set, "2026-11"), ["2026-11-12 17:30→18:30", "2026-11-26 17:30→18:30"]);
    // The opposite parity never appears.
    let other_week = parse_set(&json!({
        "interval": "every two weeks",
        "start": "2026-09-10T00:00:00-05:00",
        "availabilities": [{"dtstart": "2026-09-10T17:30:00-05:00", "dtend": "2026-09-10T18:30:00-05:00"}]
    }));
    assert_eq!(occurrences(&other_week, "2026-11"), ["2026-11-05 17:30→18:30", "2026-11-19 17:30→18:30"]);
}

#[test]
fn two_week_cycle_with_entries_in_both_weeks() {
    // Week A: Monday; week B: Wednesday of the following week.
    let set = parse_set(&json!({
        "interval": "P2W",
        "start": "2026-11-02T00:00:00-06:00",
        "availabilities": [
            {"dtstart": "2026-11-02T06:00:00-06:00", "dtend": "2026-11-02T07:00:00-06:00"},
            {"dtstart": "2026-11-11T06:00:00-06:00", "dtend": "2026-11-11T07:00:00-06:00"}
        ]
    }));
    assert_eq!(
        occurrences(&set, "2026-11"),
        ["2026-11-02 06:00→07:00", "2026-11-11 06:00→07:00", "2026-11-16 06:00→07:00", "2026-11-25 06:00→07:00", "2026-11-30 06:00→07:00"]
    );
}

#[test]
fn full_day_covers_the_whole_local_day() {
    let set = parse_set(&julia_sets()[2]);
    // Sundays through Nov 15 (until). Midnight to midnight.
    assert_eq!(
        occurrences(&set, "2026-11"),
        ["2026-11-01 00:00→11-02 00:00", "2026-11-08 00:00→11-09 00:00", "2026-11-15 00:00→11-16 00:00"]
    );
    // dtend forms: same instant, next midnight, and a missing dtend.
    for entry in [
        json!({"dtstart": "2026-11-04T09:00:00-06:00", "dtend": "2026-11-04T09:00:00-06:00", "fullDay": true}),
        json!({"dtstart": "2026-11-04T00:00:00-06:00", "dtend": "2026-11-05T00:00:00-06:00", "fullDay": true}),
        json!({"dtstart": "2026-11-04", "fullDay": true}),
    ] {
        let s = parse_set(&json!({"interval": 1, "availabilities": [entry]}));
        assert!(s.problem.is_none(), "{:?}", s.problem);
        assert_eq!(occurrences(&s, "2026-11")[0], "2026-11-04 00:00→11-05 00:00");
    }
    // A two-day full-day entry.
    let two = parse_set(&json!({"interval": 1, "availabilities": [
        {"dtstart": "2026-11-06T00:00:00-06:00", "dtend": "2026-11-07T23:59:59-06:00", "fullDay": true}]}));
    assert_eq!(occurrences(&two, "2026-11")[0], "2026-11-06 00:00→11-08 00:00");
}

#[test]
fn wall_clock_time_survives_dst_changes() {
    let utc = |ndt: NaiveDateTime| studio_local(ndt).with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%MZ").to_string();
    let tuesdays = parse_set(&julia_sets()[0]);

    // Fall back: Sun Nov 1 2026. 9:45 stays 9:45; the UTC instant moves.
    let (f, n) = month("2026-10");
    let oct = expand_set(&tuesdays, f, n);
    assert_eq!(utc(oct.last().unwrap().0), "2026-10-27T14:45Z"); // CDT
    let (f, n) = month("2026-11");
    let nov = expand_set(&tuesdays, f, n);
    assert_eq!(utc(nov[0].0), "2026-11-03T15:45Z"); // CST
    assert_eq!(nov[0].0.format("%H:%M").to_string(), "09:45");

    // Spring forward: Sun Mar 14 2027.
    let (f, n) = month("2027-03");
    let mar: Vec<String> = expand_set(&tuesdays, f, n).into_iter().map(|(s, _)| utc(s)).collect();
    assert_eq!(
        mar,
        ["2027-03-02T15:45Z", "2027-03-09T15:45Z", "2027-03-16T14:45Z", "2027-03-23T14:45Z", "2027-03-30T14:45Z"]
    );

    // On the change days themselves (Sunday classes), and a full-day block
    // that is 25h / 23h long in absolute time.
    let sundays = parse_set(&json!({"interval": "weekly", "availabilities": [
        {"dtstart": "2026-10-25T09:45:00-05:00", "dtend": "2026-10-25T10:45:00-05:00"},
        {"dtstart": "2026-10-25T00:00:00-05:00", "dtend": "2026-10-25T23:59:59-05:00", "fullDay": true}]}));
    let (f, n) = month("2026-11");
    let nov = expand_set(&sundays, f, n);
    let (day_s, day_e) = nov[0];
    assert_eq!((utc(day_s), utc(day_e)), ("2026-11-01T05:00Z".to_string(), "2026-11-02T06:00Z".to_string()));
    assert_eq!((studio_local(day_e) - studio_local(day_s)).num_hours(), 25);
    assert_eq!(utc(nov[1].0), "2026-11-01T15:45Z");
    let (f, n) = month("2027-03");
    let mar = expand_set(&sundays, f, n);
    let on_14 = |x: &&(NaiveDateTime, NaiveDateTime)| x.0.date() == d("2027-03-14");
    let day = mar.iter().filter(on_14).min().unwrap();
    assert_eq!((studio_local(day.1) - studio_local(day.0)).num_hours(), 23);
    let class = mar.iter().filter(on_14).max().unwrap();
    assert_eq!(utc(class.0), "2027-03-14T14:45Z");
}

#[test]
fn unrecognized_interval_is_recorded_not_dropped() {
    let set = parse_set(&julia_sets()[3]);
    assert_eq!(set.interval_days, None);
    assert_eq!(set.interval_raw.as_deref(), Some("\"monthly on the first Monday\""));
    assert!(set.problem.as_deref().unwrap().contains("not understood"), "{:?}", set.problem);
    assert!(occurrences(&set, "2026-11").is_empty());

    // Other problems are recorded too; readable entries still expand.
    let partial = parse_set(&json!({"interval": 1, "availabilities": [
        {"dtstart": "2026-11-03T09:45:00-06:00", "dtend": "2026-11-03T10:45:00-06:00"},
        {"dtstart": "tuesday-ish", "dtend": "later"},
        {"summary": "no times"}]}));
    let problem = partial.problem.clone().unwrap();
    assert!(problem.contains("1 entry without dtstart/dtend") && problem.contains("unreadable times"), "{problem}");
    assert_eq!(occurrences(&partial, "2026-11").len(), 4);
    let bad_until = parse_set(&json!({"interval": 1, "until": "someday", "availabilities": [
        {"dtstart": "2026-11-03T09:45:00-06:00", "dtend": "2026-11-03T10:45:00-06:00"}]}));
    assert!(bad_until.problem.as_deref().unwrap().contains("until"));
    assert!(occurrences(&bad_until, "2026-11").is_empty(), "an unreadable bound is not guessed");

    assert_eq!(uninterpreted_warning(0), None);
    assert_eq!(
        uninterpreted_warning(2).unwrap(),
        "2 availability sets from Sling couldn't be interpreted — schedule may miss unavailability; see raw pull file"
    );
    assert!(uninterpreted_warning(1).unwrap().starts_with("1 availability set from Sling couldn't"));
}

// ---------- storage + blocks ----------

#[test]
fn stores_sets_raw_and_replaces_per_user() {
    let conn = db();
    assert_eq!(replace_sets_for_user(&conn, JULIA as i64, &julia_sets()).unwrap(), 4);
    let alex = unwrap_sets(&fixture()["user_alex_wrapped"]).unwrap();
    assert_eq!(replace_sets_for_user(&conn, ALEX as i64, &alex).unwrap(), 1);

    type Row = (Option<String>, Option<String>, Option<i32>, bool, Option<String>, Option<String>);
    let rows: Vec<Row> = conn
        .prepare(
            "SELECT sling_set_id, interval_raw, interval_days, pending, until, problem
             FROM sling_availability_sets WHERE sling_user_id = ? ORDER BY id",
        )
        .unwrap()
        .query_map([JULIA], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows[0], (Some("9001".into()), Some("\"P1W\"".into()), Some(7), false, None, None));
    assert_eq!(rows[1].2, Some(14));
    assert_eq!((rows[2].2, rows[2].3), (Some(7), true));
    assert_eq!(rows[2].4.as_deref(), Some("2026-11-15T23:59:59-06:00"));
    assert_eq!(rows[3].2, None);
    assert!(rows[3].5.is_some());
    // The raw JSON round-trips.
    let raw: String = conn
        .query_row("SELECT raw_json FROM sling_availability_sets WHERE sling_set_id = '9001'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), julia_sets()[0]);

    // A second pull replaces Julia's sets and leaves Alex's alone; a set
    // naming another user is not stored under this one.
    let mut next = vec![julia_sets()[0].clone()];
    next.push(alex[0].clone());
    assert_eq!(replace_sets_for_user(&conn, JULIA as i64, &next).unwrap(), 1);
    let count = |uid: i32| -> i64 {
        conn.query_row("SELECT count(*) FROM sling_availability_sets WHERE sling_user_id = ?", [uid], |r| r.get(0))
            .unwrap()
    };
    assert_eq!((count(JULIA), count(ALEX)), (1, 1));
    // An empty response clears them.
    replace_sets_for_user(&conn, JULIA as i64, &[]).unwrap();
    assert_eq!(count(JULIA), 0);
}

#[test]
fn writes_blocks_with_sources_and_dedupes_against_calendar() {
    let conn = db();
    replace_sets_for_user(&conn, JULIA as i64, &julia_sets()).unwrap();
    // The calendar already carries Julia's Nov 10 Tuesday block (same
    // instants, written as UTC) and an unrelated leave.
    conn.execute_batch(
        "INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at) VALUES
           (1930004, 'availability', TIMESTAMPTZ '2026-11-10 15:45:00+00', TIMESTAMPTZ '2026-11-10 16:45:00+00'),
           (1930002, 'leave', TIMESTAMPTZ '2026-11-20 00:00:00-06', TIMESTAMPTZ '2026-11-22 00:00:00-06');",
    )
    .unwrap();
    let stats = write_set_blocks(&conn, "2026-11", &roster()).unwrap();
    // Tuesdays 3/10/17/24 (10th is a duplicate) + Thursdays 12/26 = 5
    // approved; Sundays 1/8/15 = 3 pending; the monthly set = 0.
    assert_eq!(stats, SetBlockStats { written: 5, pending: 3, duplicates: 1 });

    let all = blocks(&conn);
    let julia: Vec<(&str, &str)> =
        all.iter().filter(|b| b.0 == JULIA).map(|b| (b.1.as_str(), b.2.as_str())).collect();
    assert_eq!(
        julia,
        [
            ("availability_set_pending", "2026-11-01T05:00:00Z"),
            ("availability_set", "2026-11-03T15:45:00Z"),
            ("availability_set_pending", "2026-11-08T06:00:00Z"),
            ("availability", "2026-11-10T15:45:00Z"),
            ("availability_set", "2026-11-12T23:30:00Z"),
            ("availability_set_pending", "2026-11-15T06:00:00Z"),
            ("availability_set", "2026-11-17T15:45:00Z"),
            ("availability_set", "2026-11-24T15:45:00Z"),
            ("availability_set", "2026-11-26T23:30:00Z"),
        ]
    );
    // Julia's Tuesday 9:45 block exists every week of the month.
    assert_eq!(all.iter().filter(|b| b.0 == JULIA && b.2.ends_with("T15:45:00Z")).count(), 4);

    // Running it again adds nothing (every occurrence is now a duplicate).
    let again = write_set_blocks(&conn, "2026-11", &roster()).unwrap();
    assert_eq!((again.written, again.pending, again.duplicates), (0, 0, 9));
    assert_eq!(blocks(&conn).len(), all.len());

    // A teacher who is not on the roster gets no blocks.
    let only_alex: HashSet<i32> = [ALEX].into_iter().collect();
    assert_eq!(write_set_blocks(&conn, "2026-12", &only_alex).unwrap(), SetBlockStats::default());
}

#[test]
fn uninterpreted_sets_surface_as_issues() {
    let conn = db();
    replace_sets_for_user(&conn, JULIA as i64, &julia_sets()).unwrap();
    let issues = set_issues(&conn).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].teacher_name.as_deref(), Some("Julia Stone"));
    assert_eq!(issues[0].name.as_deref(), Some("First Monday"));
    assert_eq!(issues[0].interval_raw.as_deref(), Some("\"monthly on the first Monday\""));
    let month = compute_month(&conn, "2026-11").unwrap();
    assert_eq!((month.set_count, month.pending_set_count), (4, 1));
    assert_eq!(month.warnings.len(), 1);
    assert!(month.warnings[0].starts_with("1 availability set from Sling couldn't be interpreted"));
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

/// November 2026 with Julia's recurring sets, a calendar block, a leave and
/// a month of class slots (including one outside studio hours).
fn november(conn: &duckdb::Connection) {
    replace_sets_for_user(conn, JULIA as i64, &julia_sets()).unwrap();
    conn.execute_batch(
        "INSERT INTO availability_blocks (sling_user_id, source, starts_at, ends_at) VALUES
           (1930002, 'leave', TIMESTAMPTZ '2026-10-30 00:00:00-05', TIMESTAMPTZ '2026-11-03 23:59:59-06'),
           (1930001, 'availability', TIMESTAMPTZ '2026-11-04 08:45:00-06', TIMESTAMPTZ '2026-11-04 09:45:00-06'),
           (1930001, 'availability', TIMESTAMPTZ '2026-11-05 10:15:00-06', TIMESTAMPTZ '2026-11-05 12:00:00-06'),
           (1930001, 'availability', TIMESTAMPTZ '2026-11-06 20:30:00-06', TIMESTAMPTZ '2026-11-06 22:00:00-06');
         INSERT INTO proposals (id, target_month, algorithm_version, parameters, is_current)
           VALUES (1, '2026-11', 'v9', '{}', TRUE);",
    )
    .unwrap();
    write_set_blocks(conn, "2026-11", &roster()).unwrap();
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
    // Julia: recurring Tuesday 9:45 block; pending full-day Sundays.
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
    assert_eq!(propose_event_type(SOURCE_SET), "availability");
    assert_eq!(propose_event_type(SOURCE_SET_PENDING), "availability");
}
