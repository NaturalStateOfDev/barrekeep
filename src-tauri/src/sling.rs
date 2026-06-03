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
}
