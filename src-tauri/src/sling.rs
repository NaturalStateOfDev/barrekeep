//! Sling API client. Sync HTTP via ureq (matching the existing Anthropic
//! pattern in commands.rs). Endpoints documented in docs/sling-api.md.
//!
//! The PullPayload returned by pull_month() is the canonical structure
//! that pull_month_from_sling (commands.rs) writes to DuckDB and that
//! propose.py reads via stdin.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

pub const BASE_URL: &str = "https://api.getsling.com/v1";

/// Per-studio Sling identifiers, loaded at runtime from the `studio_config`
/// table (see migration 0007). Formerly compiled-in constants — externalized
/// so the shipped/public binary carries no real org identity.
#[derive(Debug, Clone, Copy)]
pub struct StudioConfig {
    pub org_id: i64,
    pub acting_user_id: i64,
    pub home_location_id: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SlingUser {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub lastname: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default, rename = "groupIds")]
    pub group_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SlingGroup {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: String, // "position", "location", etc.
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CalendarEvent {
    #[serde(default, deserialize_with = "deser_opt_i64_flex")]
    pub id: Option<i64>,
    #[serde(default, rename = "type")]
    pub kind: String, // "shift" | "leave" | "availability"
    #[serde(default)]
    pub dtstart: String, // ISO with offset
    #[serde(default)]
    pub dtend: String,
    #[serde(default)]
    pub user: Option<SlingEventUserRef>,
    #[serde(default)]
    pub users: Option<Vec<SlingEventUserRef>>,
    #[serde(default)]
    pub position: Option<SlingEventPositionRef>,
    #[serde(default)]
    pub location: Option<SlingEventLocationRef>,
    #[serde(default)]
    pub status: Option<String>, // shifts only
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SlingEventUserRef {
    #[serde(deserialize_with = "deser_i64_flex")]
    pub id: i64,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SlingEventPositionRef {
    #[serde(deserialize_with = "deser_i64_flex")]
    pub id: i64,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SlingEventLocationRef {
    #[serde(deserialize_with = "deser_i64_flex")]
    pub id: i64,
}

/// Input row from the `proposal_shifts` table, passed to build_push_specs.
#[derive(Debug, Clone)]
pub struct ProposalShiftInput {
    pub proposal_shift_id: i64,
    pub date: String,
    pub start: String,
    pub end: String,
    pub position_id: i64,
    pub user_id: Option<i64>,
    pub teacher_name: Option<String>,
    pub class_name: String,
    pub is_coteach: bool,
    pub coteach_label: Option<String>,
    pub is_dropped: bool,
}

/// A single shift to be created or verified against Sling.
/// Produced by push_to_sling (commands.rs) from the `proposal_shifts` table.
#[derive(Debug, Clone)]
pub struct PushSpec {
    pub proposal_shift_id: i64,
    pub date: String,         // "2026-06-01"
    pub start: String,        // "05:45"
    pub end: String,          // "06:45"
    pub position_id: i64,
    pub user_id: i64,
    pub class_name: String,   // display only
    pub teacher_name: String, // display only
}

// Sling returns some id fields as JSON strings (notably the top-level event
// `id` — stringified to preserve precision beyond JS's 53-bit limit), others
// as JSON numbers. Accept both shapes.
fn deser_i64_flex<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flex {
        Int(i64),
        Str(String),
    }
    match Flex::deserialize(d)? {
        Flex::Int(n) => Ok(n),
        Flex::Str(s) => s.parse::<i64>().map_err(serde::de::Error::custom),
    }
}

fn deser_opt_i64_flex<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flex {
        Int(i64),
        Str(String),
    }
    match Option::<Flex>::deserialize(d)? {
        None => Ok(None),
        Some(Flex::Int(n)) => Ok(Some(n)),
        Some(Flex::Str(s)) => s.parse::<i64>().map(Some).map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Serialize)]
pub struct PullPayload {
    pub target_month: String,              // "YYYY-MM"
    pub users: Vec<SlingUser>,             // full roster from Sling
    pub groups: Vec<SlingGroup>,           // for position-group identification
    pub month_events: Vec<CalendarEvent>,  // target month: availability + leave + shifts
    pub history_shifts: Vec<CalendarEvent>, // trailing 3 months, shifts only, home location only
}

/// Returns the set of group IDs for position-type groups only.
pub fn position_group_ids(groups: &[SlingGroup]) -> std::collections::HashSet<i64> {
    groups
        .iter()
        .filter(|g| g.kind == "position")
        .map(|g| g.id)
        .collect()
}

/// Returns a (location_id → name) map for location-type groups only.
/// Users' Sling `groupIds` include both position and location ids, so
/// intersecting against this map yields the user's location memberships.
pub fn location_name_by_id(groups: &[SlingGroup]) -> std::collections::HashMap<i64, String> {
    groups
        .iter()
        .filter(|g| g.kind == "location")
        .map(|g| (g.id, g.name.clone()))
        .collect()
}

/// Returns a comma-joined string of location names for the given user
/// group_ids, or None if the user has no location memberships. The
/// common "the barre studio " prefix is trimmed for display brevity.
pub fn compute_locations(
    group_ids: &[i64],
    names: &std::collections::HashMap<i64, String>,
) -> Option<String> {
    let mut locs: Vec<String> = group_ids
        .iter()
        .filter_map(|g| names.get(g).cloned())
        .map(|n| n.strip_prefix("the barre studio ").map(str::to_string).unwrap_or(n))
        .collect();
    if locs.is_empty() {
        return None;
    }
    locs.sort();
    Some(locs.join(", "))
}

/// Filter an event list to (home location ∪ no-location) + the given kind(s).
/// Matches scripts/sling_extract.py:is_home_teacher_event — events
/// without a location are allowed through (Sling sometimes omits the
/// field on past or planning-state shifts and on time-off events).
pub fn filter_events<'a>(
    events: &'a [CalendarEvent],
    kinds: &[&str],
    home_location_id: i64,
) -> Vec<&'a CalendarEvent> {
    events
        .iter()
        .filter(|e| {
            kinds.contains(&e.kind.as_str())
                && e.location
                    .as_ref()
                    .map_or(true, |l| l.id == home_location_id)
        })
        .collect()
}

fn http_get(token: &str, url: &str) -> Result<serde_json::Value> {
    http_get_with_query(token, url, &[])
}

/// Like http_get but routes query params through ureq's .query() method so
/// reserved characters (notably `:` in ISO datetimes and `/` in the
/// dates= separator) get percent-encoded. Matches what Python's `requests`
/// does when you pass a params dict.
fn http_get_with_query(token: &str, url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value> {
    let mut req = ureq::get(url)
        .set("Authorization", token)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .set("Origin", "https://app.getsling.com")
        .set("Referer", "https://app.getsling.com/")
        .set("Sec-Fetch-Dest", "empty")
        .set("Sec-Fetch-Mode", "cors")
        .set("Sec-Fetch-Site", "same-site")
        .set("Accept", "application/json");
    for (k, v) in query {
        req = req.query(k, v);
    }
    let resp = req.call();
    match resp {
        Ok(r) => Ok(r.into_json::<serde_json::Value>()
            .with_context(|| format!("invalid JSON from {url}"))?),
        Err(ureq::Error::Status(401, _)) => Err(anyhow!("sling-401")),
        Err(ureq::Error::Status(429, _)) => Err(anyhow!("sling-429")),
        Err(ureq::Error::Status(1010, _)) => Err(anyhow!("sling-1010")),
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            Err(anyhow!("sling-{code}: {}", body))
        }
        Err(e) => Err(anyhow!("sling-network: {e}")),
    }
}

/// Returns (startISO, endISO) for the target month, last day of month at 23:59,
/// with a -05:00 offset. Matches scripts/sling_extract.py:82-85 — Sling
/// returns empty on historical /calendar queries when the offset is omitted.
///
/// Known limitation: -05:00 is correct for Central Daylight Time only.
/// DST handling is the same TODO that's already noted in CLAUDE.md.
pub fn month_range(target_month: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = target_month.split('-').collect();
    if parts.len() != 2 { return Err(anyhow!("bad target_month: {target_month}")); }
    let year: i32 = parts[0].parse()?;
    let month: u32 = parts[1].parse()?;
    let start = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| anyhow!("invalid date"))?;
    let next = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    }.ok_or_else(|| anyhow!("invalid date"))?;
    let end = next.pred_opt().unwrap();
    Ok((format!("{start}T00:00:00-05:00"), format!("{end}T23:59:59-05:00")))
}

/// POST viewdates/cachedates windows for the target month. These are
/// cache-invalidation hints Sling's server uses; we reproduce the web
/// client's padding (prev day .. first-of-next-month + 4 days, cachedates
/// one day wider each side). NB: offset is "-0500" (no colon) here, unlike
/// the calendar `dates=` param which uses "-05:00". Matches
/// scripts/push_to_sling.py VIEWDATES/CACHEDATES for June 2026.
pub fn view_cache_dates(month: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = month.split('-').collect();
    if parts.len() != 2 { return Err(anyhow!("bad month: {month}")); }
    let year: i32 = parts[0].parse()?;
    let mon: u32 = parts[1].parse()?;
    let first = chrono::NaiveDate::from_ymd_opt(year, mon, 1)
        .ok_or_else(|| anyhow!("invalid date"))?;
    let next_first = if mon == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, mon + 1, 1)
    }.ok_or_else(|| anyhow!("invalid date"))?;
    // NB: "-0500" (no colon) — Sling's viewdates/cachedates format. Do NOT
    // change to "-05:00"; that colon-form is only for the calendar dates= param.
    let fmt = |d: chrono::NaiveDate| format!("{d}T00:00:00-0500");
    let view_start = first - chrono::Duration::days(1);
    let view_end = next_first + chrono::Duration::days(4);
    let cache_start = view_start - chrono::Duration::days(1);
    let cache_end = view_end + chrono::Duration::days(1);
    Ok((
        format!("{}/{}", fmt(view_start), fmt(view_end)),
        format!("{}/{}", fmt(cache_start), fmt(cache_end)),
    ))
}

/// Split a Sling dtstart ("2026-06-01T05:45:00-05:00") into (date, "HH:MM").
pub fn split_dt(dt: &str) -> (String, String) {
    if let Some((date, time)) = dt.split_once('T') {
        (date.to_string(), time.chars().take(5).collect())
    } else {
        (dt.chars().take(10).collect(), "00:00".to_string())
    }
}

/// Stable dedupe key: "date|HH:MM|user_id|position_id|location_id".
pub fn spec_fingerprint(s: &PushSpec, home_location_id: i64) -> String {
    format!("{}|{}|{}|{}|{}", s.date, s.start, s.user_id, s.position_id, home_location_id)
}

/// Build the set of fingerprints already present at the home location.
/// Only planning + published shifts count (matches push_to_sling.py).
pub fn existing_fingerprints(events: &[CalendarEvent], home_location_id: i64) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for ev in events {
        if ev.kind != "shift" { continue; }
        // Unlike filter_events (which lets location-less events through), we
        // require an explicit home-location match here — matches
        // push_to_sling.py's existing_shifts_at_home. A shift returned without
        // a location can't be confirmed as home, so it's conservatively not
        // counted as a duplicate; the push would re-attempt it, and re-push is
        // idempotent. Our own created shifts always echo back their location.
        let Some(loc) = ev.location.as_ref() else { continue; };
        if loc.id != home_location_id { continue; }
        match ev.status.as_deref() {
            Some("planning") | Some("published") => {}
            _ => continue,
        }
        let Some(user) = ev.user.as_ref() else { continue; };
        let Some(pos) = ev.position.as_ref() else { continue; };
        let (date, hhmm) = split_dt(&ev.dtstart);
        out.insert(format!("{}|{}|{}|{}|{}", date, hhmm, user.id, pos.id, home_location_id));
    }
    out
}

/// Build POST specs from proposal rows. Dropped shifts are skipped. A
/// non-dropped shift with no teacher is a hard error (it can't become a
/// valid Sling shift). Co-teach rows expand into one spec per teacher named
/// in `coteach_label`, resolved through `name_to_id` (display_name -> id).
pub fn build_push_specs(
    inputs: &[ProposalShiftInput],
    name_to_id: &std::collections::HashMap<String, i64>,
) -> Result<Vec<PushSpec>, String> {
    let mut specs = Vec::new();
    for inp in inputs {
        if inp.is_dropped { continue; }
        if inp.is_coteach {
            let label = inp.coteach_label.as_deref().unwrap_or("");
            let names: Vec<&str> = label.split(" + ").map(str::trim).filter(|n| !n.is_empty()).collect();
            if names.is_empty() {
                return Err(format!("co-teach shift on {} {} has no teacher names", inp.date, inp.start));
            }
            for name in names {
                let uid = name_to_id.get(name).ok_or_else(|| format!(
                    "co-teach shift on {} {} references unknown teacher '{}'", inp.date, inp.start, name))?;
                specs.push(PushSpec {
                    proposal_shift_id: inp.proposal_shift_id, date: inp.date.clone(), start: inp.start.clone(),
                    end: inp.end.clone(), position_id: inp.position_id, user_id: *uid,
                    class_name: inp.class_name.clone(), teacher_name: name.to_string(),
                });
            }
        } else {
            let uid = inp.user_id.ok_or_else(|| format!(
                "shift on {} {} ({}) has no teacher assigned — resolve it before pushing",
                inp.date, inp.start, inp.class_name))?;
            specs.push(PushSpec {
                proposal_shift_id: inp.proposal_shift_id, date: inp.date.clone(), start: inp.start.clone(),
                end: inp.end.clone(), position_id: inp.position_id, user_id: uid,
                class_name: inp.class_name.clone(),
                teacher_name: inp.teacher_name.clone().unwrap_or_default(),
            });
        }
    }
    Ok(specs)
}

pub fn pull_month(token: &str, target_month: &str, cfg: &StudioConfig) -> Result<PullPayload> {
    let _ = month_range(target_month)?;
    let org_id = cfg.org_id;
    let acting_user_id = cfg.acting_user_id;

    let users_url = format!("{BASE_URL}/users/concise");
    let users_doc = http_get(token, &users_url)?;
    let users: Vec<SlingUser> = users_doc.get("users")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("users array missing"))?
        .iter()
        .filter_map(|u| serde_json::from_value(u.clone()).ok())
        .collect();

    let groups_url = format!("{BASE_URL}/groups");
    let groups_doc = http_get(token, &groups_url)?;
    let groups: Vec<SlingGroup> = groups_doc.as_array()
        .ok_or_else(|| anyhow!("groups not array"))?
        .iter()
        .filter_map(|g| serde_json::from_value(g.clone()).ok())
        .collect();

    let (start, end) = month_range(target_month)?;
    let nonce = chrono::Utc::now().timestamp_millis();
    let cal_url = format!("{BASE_URL}/{org_id}/calendar/{org_id}/users/{acting_user_id}");
    let dates_param = format!("{start}/{end}");
    let nonce_str = nonce.to_string();
    let cal_doc = http_get_with_query(token, &cal_url, &[
        ("dates", &dates_param),
        ("user-fields", "id"),
        ("nonce", &nonce_str),
    ])?;
    let cal_arr = cal_doc.as_array().ok_or_else(|| anyhow!("calendar not array"))?;
    let raw_month_total = cal_arr.len();
    let month_events: Vec<CalendarEvent> = cal_arr
        .iter()
        .filter_map(|e| serde_json::from_value(e.clone()).ok())
        .collect();
    eprintln!(
        "[sling] month /calendar {dates_param}: {raw_month_total} raw events"
    );

    let (h_start_y, h_start_m) = {
        let (y, m): (i32, u32) = {
            let p: Vec<&str> = target_month.split('-').collect();
            (p[0].parse()?, p[1].parse()?)
        };
        let mut y2 = y;
        let mut m2 = m as i32 - 3;
        while m2 < 1 { m2 += 12; y2 -= 1; }
        (y2, m2 as u32)
    };
    // -05:00 offset matches month_range() / scripts/sling_extract.py; without
    // it, Sling returns empty for historical /calendar queries.
    let hist_start_iso = format!("{h_start_y:04}-{h_start_m:02}-01T00:00:00-05:00");
    let hist_end_iso = start.clone();
    let nonce2 = chrono::Utc::now().timestamp_millis();
    let hist_url = format!("{BASE_URL}/{org_id}/calendar/{org_id}/users/{acting_user_id}");
    let hist_dates_param = format!("{hist_start_iso}/{hist_end_iso}");
    let hist_nonce_str = nonce2.to_string();
    let hist_doc = http_get_with_query(token, &hist_url, &[
        ("dates", &hist_dates_param),
        ("user-fields", "id"),
        ("nonce", &hist_nonce_str),
    ])?;
    let hist_arr = hist_doc.as_array()
        .ok_or_else(|| anyhow!("history calendar not array"))?;
    let raw_total = hist_arr.len();
    let parsed: Vec<CalendarEvent> = hist_arr
        .iter()
        .filter_map(|e| serde_json::from_value(e.clone()).ok())
        .collect();
    let parsed_count = parsed.len();
    let history_shifts: Vec<CalendarEvent> = parsed
        .into_iter()
        .filter(|e: &CalendarEvent|
            e.kind == "shift"
            && e.location.as_ref().map_or(true, |l| l.id == cfg.home_location_id)
        )
        .collect();
    eprintln!(
        "[sling] history /calendar {hist_dates_param}: {raw_total} raw, \
         {parsed_count} parsed, {} after shift+home location filter",
        history_shifts.len()
    );
    // One-shot diag: dump the first raw event so we can see its actual shape.
    if let Some(first) = hist_arr.first() {
        let s = serde_json::to_string(first).unwrap_or_default();
        let short: String = s.chars().take(800).collect();
        eprintln!("[sling] first raw history event: {short}");
    }
    // And the kinds distribution to spot field-name drift.
    {
        let mut kind_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for e in hist_arr {
            let k = e.get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("<missing type>")
                .to_string();
            *kind_counts.entry(k).or_insert(0) += 1;
        }
        eprintln!("[sling] history event type breakdown: {kind_counts:?}");
    }

    Ok(PullPayload {
        target_month: target_month.to_string(),
        users,
        groups,
        month_events,
        history_shifts,
    })
}

/// The create-shift POST body. `users` is an array on POST (PUT uses
/// singular `user`); `status` is always the literal "planning" — this app
/// never publishes. dtstart/dtend are naive local strings; Sling applies the
/// timezone on echo. See docs/sling-api.md.
pub fn build_shift_body(s: &PushSpec, home_location_id: i64) -> serde_json::Value {
    serde_json::json!({
        "location": { "id": home_location_id },
        "dtstart": format!("{}T{}", s.date, s.start),
        "dtend": format!("{}T{}", s.date, s.end),
        "users": [{ "id": s.user_id }],
        "slots": 1,
        "position": { "id": s.position_id },
        "status": "planning",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn month_range_returns_correct_bounds() {
        let (s, e) = month_range("2026-06").unwrap();
        assert_eq!(s, "2026-06-01T00:00:00-05:00");
        assert_eq!(e, "2026-06-30T23:59:59-05:00");
        let (s2, e2) = month_range("2026-12").unwrap();
        assert_eq!(s2, "2026-12-01T00:00:00-05:00");
        assert_eq!(e2, "2026-12-31T23:59:59-05:00");
    }

    #[test]
    fn parses_sling_discovery_users() {
        let raw = fs::read_to_string("test_fixtures/sling_discovery_sample.json")
            .expect("fixture present (see Task 4 Step 1)");
        let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let users_arr = doc
            .pointer("/users/users")
            .and_then(|v| v.as_array())
            .expect("users.users array");
        let users: Vec<SlingUser> = users_arr
            .iter()
            .map(|u| serde_json::from_value(u.clone()).expect("user"))
            .collect();
        assert!(users.len() > 5, "expected multiple users in fixture");
        let lead = users
            .iter()
            .find(|u| u.id == 1001)
            .expect("Teacher A in fixture");
        assert!(!lead.group_ids.is_empty());
    }

    #[test]
    fn parses_sling_discovery_groups_and_filters_positions() {
        let raw = fs::read_to_string("test_fixtures/sling_discovery_sample.json").unwrap();
        let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let groups_arr = doc
            .get("groups")
            .and_then(|v| v.as_array())
            .expect("groups array");
        let groups: Vec<SlingGroup> = groups_arr
            .iter()
            .map(|g| serde_json::from_value(g.clone()).unwrap())
            .collect();
        let pos_ids = position_group_ids(&groups);
        assert!(pos_ids.contains(&29470407), "Empower position group present");
        assert!(pos_ids.contains(&29303965), "Classic position group present");
    }

    #[test]
    fn view_cache_dates_reproduce_june_window() {
        let (view, cache) = view_cache_dates("2026-06").unwrap();
        // Matches the constants the working push_to_sling.py used for June 2026.
        assert_eq!(view, "2026-05-31T00:00:00-0500/2026-07-05T00:00:00-0500");
        assert_eq!(cache, "2026-05-30T00:00:00-0500/2026-07-06T00:00:00-0500");
    }

    #[test]
    fn view_cache_dates_handles_december_year_rollover() {
        let (view, cache) = view_cache_dates("2026-12").unwrap();
        assert_eq!(view, "2026-11-30T00:00:00-0500/2027-01-05T00:00:00-0500");
        assert_eq!(cache, "2026-11-29T00:00:00-0500/2027-01-06T00:00:00-0500");
    }

    #[test]
    fn split_dt_extracts_date_and_hhmm() {
        assert_eq!(split_dt("2026-06-01T05:45:00-05:00"), ("2026-06-01".into(), "05:45".into()));
        assert_eq!(split_dt("2026-06-01"), ("2026-06-01".into(), "00:00".into()));
    }

    #[test]
    fn existing_fingerprints_filters_and_keys_correctly() {
        let events = vec![
            // home shift, planning -> included
            CalendarEvent {
                id: Some(1),
                kind: "shift".into(),
                dtstart: "2026-06-01T05:45:00-05:00".into(),
                dtend: "2026-06-01T06:45:00-05:00".into(),
                user: Some(SlingEventUserRef { id: 1001 }),
                users: None,
                position: Some(SlingEventPositionRef { id: 29470407 }),
                location: Some(SlingEventLocationRef { id: 5 }),
                status: Some("planning".into()),
            },
            // wrong location -> excluded
            CalendarEvent {
                id: Some(2),
                kind: "shift".into(),
                dtstart: "2026-06-01T05:45:00-05:00".into(),
                dtend: "2026-06-01T06:45:00-05:00".into(),
                user: Some(SlingEventUserRef { id: 1002 }),
                users: None,
                position: Some(SlingEventPositionRef { id: 29470407 }),
                location: Some(SlingEventLocationRef { id: 999 }),
                status: Some("planning".into()),
            },
            // leave event -> excluded
            CalendarEvent {
                id: Some(3),
                kind: "leave".into(),
                dtstart: "2026-06-02T00:00:00-05:00".into(),
                dtend: "".into(),
                user: Some(SlingEventUserRef { id: 1001 }),
                users: None,
                position: None,
                location: Some(SlingEventLocationRef { id: 5 }),
                status: None,
            },
            // published home shift -> included (published counts as present)
            CalendarEvent {
                id: Some(4),
                kind: "shift".into(),
                dtstart: "2026-06-03T09:00:00-05:00".into(),
                dtend: "2026-06-03T10:00:00-05:00".into(),
                user: Some(SlingEventUserRef { id: 1003 }),
                users: None,
                position: Some(SlingEventPositionRef { id: 29303965 }),
                location: Some(SlingEventLocationRef { id: 5 }),
                status: Some("published".into()),
            },
        ];
        let fp = existing_fingerprints(&events, 5);
        assert_eq!(fp.len(), 2);
        assert!(fp.contains("2026-06-01|05:45|1001|29470407|5"));
        assert!(fp.contains("2026-06-03|09:00|1003|29303965|5"));
    }

    #[test]
    fn build_push_specs_expands_coteach_and_skips_dropped() {
        let mut name_to_id = std::collections::HashMap::new();
        name_to_id.insert("Teacher A".to_string(), 1001i64);
        name_to_id.insert("Teacher E".to_string(), 1005i64);
        let inputs = vec![
            ProposalShiftInput { proposal_shift_id: 10, date: "2026-06-01".into(), start: "05:45".into(),
                end: "06:45".into(), position_id: 29470407, user_id: Some(1001), teacher_name: Some("Teacher A".into()),
                class_name: "Empower".into(), is_coteach: false, coteach_label: None, is_dropped: false },
            ProposalShiftInput { proposal_shift_id: 11, date: "2026-06-02".into(), start: "09:00".into(),
                end: "10:00".into(), position_id: 29303965, user_id: Some(1001), teacher_name: Some("Teacher A".into()),
                class_name: "Classic".into(), is_coteach: true, coteach_label: Some("Teacher A + Teacher E".into()), is_dropped: false },
            ProposalShiftInput { proposal_shift_id: 12, date: "2026-06-03".into(), start: "09:00".into(),
                end: "10:00".into(), position_id: 29303965, user_id: None, teacher_name: None,
                class_name: "Classic".into(), is_coteach: false, coteach_label: None, is_dropped: true },
        ];
        let specs = build_push_specs(&inputs, &name_to_id).unwrap();
        assert_eq!(specs.len(), 3); // 1 normal + 2 from co-teach + 0 dropped
        let coteach_ids: Vec<i64> = specs.iter().filter(|s| s.proposal_shift_id == 11).map(|s| s.user_id).collect();
        assert_eq!(coteach_ids, vec![1001, 1005]);
    }

    #[test]
    fn build_push_specs_errors_on_unassigned() {
        let name_to_id = std::collections::HashMap::new();
        let inputs = vec![ProposalShiftInput { proposal_shift_id: 20, date: "2026-06-01".into(), start: "05:45".into(),
            end: "06:45".into(), position_id: 29470407, user_id: None, teacher_name: None,
            class_name: "Empower".into(), is_coteach: false, coteach_label: None, is_dropped: false }];
        let e = build_push_specs(&inputs, &name_to_id).unwrap_err();
        assert!(e.contains("no teacher"), "got: {e}");
    }

    #[test]
    fn build_push_specs_errors_on_unknown_coteach_name() {
        let mut name_to_id = std::collections::HashMap::new();
        name_to_id.insert("Teacher A".to_string(), 1001i64);
        let inputs = vec![ProposalShiftInput { proposal_shift_id: 30, date: "2026-06-02".into(), start: "09:00".into(),
            end: "10:00".into(), position_id: 29303965, user_id: Some(1001), teacher_name: Some("Teacher A".into()),
            class_name: "Classic".into(), is_coteach: true, coteach_label: Some("Teacher A + Ghost".into()), is_dropped: false }];
        let e = build_push_specs(&inputs, &name_to_id).unwrap_err();
        assert!(e.contains("Ghost"), "got: {e}");
    }

    #[test]
    fn build_shift_body_matches_sling_contract() {
        let s = PushSpec { proposal_shift_id: 1, date: "2026-06-01".into(), start: "05:45".into(),
            end: "06:45".into(), position_id: 29470407, user_id: 1001,
            class_name: "Empower".into(), teacher_name: "Teacher A".into() };
        let body = build_shift_body(&s, 5);
        assert_eq!(body["dtstart"], "2026-06-01T05:45");
        assert_eq!(body["dtend"], "2026-06-01T06:45");
        assert_eq!(body["status"], "planning");
        assert_eq!(body["slots"], 1);
        assert_eq!(body["location"]["id"], 5);
        assert_eq!(body["position"]["id"], 29470407);
        // users is an ARRAY on POST (not singular `user`)
        assert_eq!(body["users"][0]["id"], 1001);
        assert!(body.get("user").is_none());
    }
}
