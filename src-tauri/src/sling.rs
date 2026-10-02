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
    pub class_name: String,
    pub is_coteach: bool,
    pub coteach_label: Option<String>,
    pub is_dropped: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredLocation { pub id: i64, pub name: String }

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredStudio {
    pub org_id: i64,
    pub acting_user_id: i64,
    pub acting_user_name: String,
    /// Human-readable org name when the session response carries one ("" otherwise).
    pub org_name: String,
    pub locations: Vec<DiscoveredLocation>,
}

/// Read a number-or-string JSON value as i64.
fn json_i64(v: &serde_json::Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// Extract (acting_user_id, name, org_id_if_present) from an account/session
/// response. Sling's exact shape is undocumented, so org-id lookup is tolerant;
/// callers fall back to the login-URL org hint when it's absent.
pub fn parse_session(v: &serde_json::Value) -> Result<(i64, String, Option<i64>)> {
    let user = v.get("user").ok_or_else(|| anyhow!("session response has no user"))?;
    let uid = user.get("id").and_then(json_i64)
        .ok_or_else(|| anyhow!("session user has no id"))?;
    let name = user.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
    let org = ["org", "organization"].iter()
        .find_map(|k| v.get(*k).and_then(|o| o.get("id")).and_then(json_i64))
        .or_else(|| v.get("orgId").and_then(json_i64))
        .or_else(|| user.get("orgId").and_then(json_i64))
        .or_else(|| user.get("org").and_then(|o| o.get("id")).and_then(json_i64));
    Ok((uid, name, org))
}

/// The org's display name from an account/session response, if present.
/// Only trusted when the response's org id matches `org_id`.
pub fn session_org_name(v: &serde_json::Value, org_id: i64) -> String {
    ["org", "organization"].iter()
        .filter_map(|k| v.get(*k))
        .find(|o| o.get("id").and_then(json_i64) == Some(org_id))
        .and_then(|o| o.get("name").and_then(|n| n.as_str()))
        .unwrap_or("")
        .to_string()
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
        // A non-numeric id (Sling documents availability-event ids as
        // strings) must not fail the whole event: an availability/leave
        // event needs no id, and dropping it would hide blocked time.
        Some(Flex::Str(s)) => Ok(s.parse::<i64>().ok()),
    }
}

#[derive(Debug, Serialize)]
pub struct PullPayload {
    pub target_month: String,              // "YYYY-MM"
    pub users: Vec<SlingUser>,             // full roster from Sling
    pub groups: Vec<SlingGroup>,           // for position-group identification
    pub month_events: Vec<CalendarEvent>,  // target month: availability + leave + shifts
    pub history_shifts: Vec<CalendarEvent>, // trailing 3 months, shifts only, home location only
    /// Recurring availability sets per teacher (GET /availability?userId=…).
    pub availability: AvailabilityFetch,
    /// Target-month calendar events that could not be parsed at all.
    pub month_events_unparsed: usize,
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
/// Matches scripts/legacy/sling_extract.py:is_home_teacher_event — events
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
                    .is_none_or(|l| l.id == home_location_id)
        })
        .collect()
}

fn http_get(token: &str, url: &str) -> Result<serde_json::Value> {
    http_get_with_query(token, url, &[])
}

/// A ureq agent that surfaces non-2xx responses as `Ok` instead of an `Err`, so
/// we can read the body and translate Sling's status codes ourselves. ureq 3
/// otherwise collapses 4xx/5xx into `Error::StatusCode` and drops the body.
fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent()
}

/// Map a ureq 3 response into parsed JSON, translating the statuses the command
/// layer keys on (sling-401 = token expired, sling-429 = rate limit, sling-1010
/// = Cloudflare block) into sentinel errors.
fn map_sling_response(
    resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    url: &str,
) -> Result<serde_json::Value> {
    match resp {
        Ok(mut r) => match r.status().as_u16() {
            401 => Err(anyhow!("sling-401")),
            429 => Err(anyhow!("sling-429")),
            1010 => Err(anyhow!("sling-1010")),
            c if (200..=299).contains(&c) => r
                .body_mut()
                .read_json::<serde_json::Value>()
                .with_context(|| format!("invalid JSON from {url}")),
            c => {
                let body = r.body_mut().read_to_string().unwrap_or_default();
                Err(anyhow!("sling-{c}: {body}"))
            }
        },
        Err(e) => Err(anyhow!("sling-network: {e}")),
    }
}

/// Like http_get but routes query params through ureq's .query() method so
/// reserved characters (notably `:` in ISO datetimes and `/` in the
/// dates= separator) get percent-encoded. Matches what Python's `requests`
/// does when you pass a params dict.
fn http_get_with_query(token: &str, url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value> {
    let mut req = http_agent()
        .get(url)
        .header("Authorization", token)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Origin", "https://app.getsling.com")
        .header("Referer", "https://app.getsling.com/")
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-site")
        .header("Accept", "application/json");
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    map_sling_response(req.call(), url)
}

/// POST JSON with browser-like headers + percent-encoded query params.
/// Returns parsed JSON on 2xx; maps known statuses to sentinel errors that
/// the command layer recognizes (sling-401, sling-429, sling-1010).
fn http_post(token: &str, url: &str, query: &[(&str, &str)], body: &serde_json::Value) -> Result<serde_json::Value> {
    let mut req = http_agent()
        .post(url)
        .header("Authorization", token)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Origin", "https://app.getsling.com")
        .header("Referer", "https://app.getsling.com/")
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-site")
        .header("Accept", "application/json, text/plain, */*");
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    map_sling_response(req.send_json(body), url)
}

const PUSH_MAX_RETRIES: u32 = 3;
const PUSH_RATE_LIMIT_BACKOFF_SECS: u64 = 30;

/// Create one planning shift. Retries on 429 up to PUSH_MAX_RETRIES with
/// linear backoff (30s, 60s, 90s). Returns the created Sling shift id.
/// Propagates "sling-401" unchanged so the caller can abort the whole run.
pub fn push_shift(token: &str, cfg: &StudioConfig, s: &PushSpec, viewdates: &str, cachedates: &str) -> Result<i64> {
    let url = format!("{BASE_URL}/{}/shifts", cfg.org_id);
    let body = build_shift_body(s, cfg.home_location_id);
    let query: [(&str, &str); 5] = [
        ("user-fields", "id"),
        ("checkRestBreakConflicts", "true"),
        ("viewdates", viewdates),
        ("cachedates", cachedates),
        ("checkConsecutiveWorkDaysConflicts", "true"),
    ];
    let mut last_err = anyhow!("push_shift: no attempts");
    for attempt in 1..=PUSH_MAX_RETRIES {
        match http_post(token, &url, &query, &body) {
            Ok(resp) => {
                // Responses are always arrays; unwrap [0]. id may be string or int.
                let obj = resp.as_array().and_then(|a| a.first()).cloned().unwrap_or(resp);
                let id = obj.get("id")
                    .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                    .ok_or_else(|| anyhow!("create response missing id: {obj}"))?;
                return Ok(id);
            }
            Err(e) if e.to_string() == "sling-429" => {
                last_err = e;
                std::thread::sleep(std::time::Duration::from_secs(PUSH_RATE_LIMIT_BACKOFF_SECS * attempt as u64));
                continue;
            }
            Err(e) => return Err(e), // includes sling-401 -> caller aborts
        }
    }
    Err(anyhow!("create failed after {PUSH_MAX_RETRIES} retries: {last_err}"))
}

/// Result of a DELETE /shifts/{id}. A 404 means the shift is already gone
/// (deleted in Sling's UI, or by an earlier run) — not an error for sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    NotFound,
}

/// DELETE with browser-like headers + percent-encoded query params. Sling
/// answers 204 with an EMPTY body, so unlike GET/POST the 2xx branch must not
/// parse JSON. Ported from scripts/legacy/rollback_push.py (method, URL, headers).
fn http_delete(token: &str, url: &str, query: &[(&str, &str)]) -> Result<DeleteOutcome> {
    let mut req = http_agent()
        .delete(url)
        .header("Authorization", token)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Origin", "https://app.getsling.com")
        .header("Referer", "https://app.getsling.com/")
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-site")
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "en-US,en;q=0.9");
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    match req.call() {
        Ok(mut r) => match r.status().as_u16() {
            401 => Err(anyhow!("sling-401")),
            404 => Ok(DeleteOutcome::NotFound),
            429 => Err(anyhow!("sling-429")),
            1010 => Err(anyhow!("sling-1010")),
            c if (200..=299).contains(&c) => Ok(DeleteOutcome::Deleted),
            c => {
                let body = r.body_mut().read_to_string().unwrap_or_default();
                Err(anyhow!("sling-{c}: {body}"))
            }
        },
        Err(e) => Err(anyhow!("sling-network: {e}")),
    }
}

/// Delete one shift. Only ever called by push_sync for shifts this app
/// created and just verified (planning, unmodified). Same 429 policy as
/// push_shift: up to PUSH_MAX_RETRIES with 30s/60s/90s backoff; sling-401
/// propagates so the caller aborts the run.
pub fn delete_shift(
    token: &str,
    cfg: &StudioConfig,
    shift_id: i64,
    viewdates: &str,
    cachedates: &str,
) -> Result<DeleteOutcome> {
    let url = format!("{BASE_URL}/{}/shifts/{shift_id}", cfg.org_id);
    let query: [(&str, &str); 2] = [("viewdates", viewdates), ("cachedates", cachedates)];
    let mut last_err = anyhow!("delete_shift: no attempts");
    for attempt in 1..=PUSH_MAX_RETRIES {
        match http_delete(token, &url, &query) {
            Ok(o) => return Ok(o),
            Err(e) if e.to_string() == "sling-429" => {
                last_err = e;
                std::thread::sleep(std::time::Duration::from_secs(PUSH_RATE_LIMIT_BACKOFF_SECS * attempt as u64));
            }
            Err(e) => return Err(e),
        }
    }
    Err(anyhow!("delete failed after {PUSH_MAX_RETRIES} retries: {last_err}"))
}

/// The assigned user of a calendar event: singular `user` (responses), else
/// the first of `users`.
pub fn event_user_id(ev: &CalendarEvent) -> Option<i64> {
    ev.user.as_ref().map(|u| u.id)
        .or_else(|| ev.users.as_ref().and_then(|v| v.first()).map(|u| u.id))
}

/// GET /v1/users/concise → roster (with group memberships).
pub fn fetch_users(token: &str) -> Result<Vec<SlingUser>> {
    let doc = http_get(token, &format!("{BASE_URL}/users/concise"))?;
    // Hard-error on a missing/malformed users array rather than returning an
    // empty roster — an empty list would make sync_roster deactivate every
    // teacher. Matches pull_month's guard.
    let arr = doc.get("users").and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("users array missing"))?;
    Ok(arr.iter().filter_map(|u| serde_json::from_value(u.clone()).ok()).collect())
}

/// GET /v1/groups → position + location groups.
pub fn fetch_groups(token: &str) -> Result<Vec<SlingGroup>> {
    let doc = http_get(token, &format!("{BASE_URL}/groups"))?;
    Ok(doc.as_array().ok_or_else(|| anyhow!("groups not array"))?
        .iter().filter_map(|g| serde_json::from_value(g.clone()).ok()).collect())
}

/// Fetch the target month's calendar events (push dedupe). Mirrors the
/// pull's calendar GET: studio-offset dates, percent-encoded, nonce — paged
/// (see `fetch_calendar_paged`).
pub fn fetch_calendar(token: &str, cfg: &StudioConfig, month: &str) -> Result<Vec<CalendarEvent>> {
    let mut session = PullSession::live(token);
    fetch_calendar_in(&mut session, cfg, month).map(|(events, _)| events)
}

/// `fetch_calendar` on a caller-owned session (shared pacing + audit).
/// Returns the parsed events and how many raw events failed to parse.
pub fn fetch_calendar_in(
    session: &mut PullSession<'_>,
    cfg: &StudioConfig,
    month: &str,
) -> Result<(Vec<CalendarEvent>, usize)> {
    let (start, end) = month_range(month)?;
    let raw = fetch_calendar_paged(session, cfg, &format!("{start}/{end}"), &format!("calendar {month}"))?;
    Ok(parse_events(&raw))
}

/// Parse raw calendar events, returning (parsed, number that failed).
pub fn parse_events(raw: &[serde_json::Value]) -> (Vec<CalendarEvent>, usize) {
    let parsed: Vec<CalendarEvent> =
        raw.iter().filter_map(|e| serde_json::from_value(e.clone()).ok()).collect();
    let dropped = raw.len() - parsed.len();
    (parsed, dropped)
}

// ============================================================
// Pull session: one paced, audited stream of GETs
// ============================================================

/// One GET as it went over the wire, for the raw-pull audit file. Never
/// holds request headers — the bearer token cannot end up in the file.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub label: String,
    pub url: String,
    pub query: Vec<(String, String)>,
    /// "ok" or the error text ("sling-404: …").
    pub outcome: String,
    pub response: serde_json::Value,
}

type GetFn<'a> = dyn FnMut(&str, &[(&str, &str)]) -> Result<serde_json::Value> + 'a;

/// Pause between consecutive GETs of one pull, and the longer pause after
/// every `GET_BATCH`th — the same cadence push_sync uses for writes (Sling
/// limits at ~20 requests/minute).
const GET_INTRA_DELAY_SECS: u64 = 1;
const GET_INTER_DELAY_SECS: u64 = 10;
const GET_BATCH: u32 = 10;

/// A sequence of Sling GETs sharing one pace (1s apart, 10s after every
/// 10th), one 429 policy (30s/60s backoff, three tries) and one audit trail.
/// Tests build it over a closure (`PullSession::with`) — no network, no
/// sleeping.
pub struct PullSession<'a> {
    get: Box<GetFn<'a>>,
    sleep: fn(std::time::Duration),
    calls: u32,
    pub audit: Vec<AuditEntry>,
}

impl<'a> PullSession<'a> {
    pub fn live(token: &'a str) -> Self {
        PullSession {
            get: Box::new(move |url, query| http_get_with_query(token, url, query)),
            sleep: std::thread::sleep,
            calls: 0,
            audit: Vec::new(),
        }
    }

    /// A session over a fake transport that never sleeps (tests).
    #[cfg(test)]
    pub fn with(get: impl FnMut(&str, &[(&str, &str)]) -> Result<serde_json::Value> + 'a) -> Self {
        PullSession { get: Box::new(get), sleep: |_| {}, calls: 0, audit: Vec::new() }
    }

    /// Requests made so far (429 retries included).
    #[cfg(test)]
    pub fn calls(&self) -> u32 {
        self.calls
    }

    fn pace(&mut self) {
        if self.calls > 0 {
            let secs = if self.calls.is_multiple_of(GET_BATCH) { GET_INTER_DELAY_SECS } else { GET_INTRA_DELAY_SECS };
            (self.sleep)(std::time::Duration::from_secs(secs));
        }
        self.calls += 1;
    }

    /// One paced GET. `record` adds the raw response to the audit trail
    /// (calendar pages and availability responses; not the roster, which
    /// carries teachers' contact details).
    pub fn get(
        &mut self,
        label: &str,
        url: &str,
        query: &[(&str, &str)],
        record: bool,
    ) -> Result<serde_json::Value> {
        let mut attempt = 0u32;
        let result = loop {
            self.pace();
            attempt += 1;
            match (self.get)(url, query) {
                Err(e) if e.to_string() == "sling-429" && attempt < PUSH_MAX_RETRIES => {
                    (self.sleep)(std::time::Duration::from_secs(
                        PUSH_RATE_LIMIT_BACKOFF_SECS * attempt as u64,
                    ));
                }
                other => break other,
            }
        };
        if record {
            self.audit.push(AuditEntry {
                label: label.to_string(),
                url: url.to_string(),
                query: query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                outcome: match &result {
                    Ok(_) => "ok".to_string(),
                    Err(e) => e.to_string(),
                },
                response: result.as_ref().cloned().unwrap_or(serde_json::Value::Null),
            });
        }
        result
    }
}

// ============================================================
// Calendar paging
// ============================================================

/// Events asked for per calendar page.
pub const CALENDAR_PAGE_SIZE: usize = 500;
/// Hard stop: 20 pages = 10,000 events, far beyond one studio's quarter.
const CALENDAR_MAX_PAGES: usize = 20;

/// Identity of a raw event for cross-page de-duplication.
fn event_key(e: &serde_json::Value) -> String {
    match e.get("id") {
        Some(serde_json::Value::String(s)) if !s.is_empty() => format!("id:{s}"),
        Some(serde_json::Value::Number(n)) => format!("id:{n}"),
        _ => e.to_string(),
    }
}

/// GET /calendar for a date range, following `pageSize`/`page`.
///
/// Sling's spec lists both params on the calendar endpoint but documents
/// neither a default page size nor whether `page` counts from 0 or 1, so the
/// loop assumes nothing:
///   - page 0 is requested first; if Sling rejects the paging params, the
///     request is repeated without them (the pre-paging behaviour);
///   - a response LARGER than the page size means Sling ignores paging and
///     already sent everything — stop;
///   - otherwise keep requesting pages, de-duplicating by event id, until a
///     page is empty or two pages in a row add nothing new. One repeat is
///     tolerated because with 1-based paging page 0 and page 1 are the same
///     page. "Short page = last page" is deliberately NOT trusted: a server
///     cap below our page size would look identical and silently truncate.
pub fn fetch_calendar_paged(
    session: &mut PullSession<'_>,
    cfg: &StudioConfig,
    dates: &str,
    label: &str,
) -> Result<Vec<serde_json::Value>> {
    let url = format!("{BASE_URL}/{}/calendar/{}/users/{}", cfg.org_id, cfg.org_id, cfg.acting_user_id);
    let page_size = CALENDAR_PAGE_SIZE.to_string();
    let as_array = |doc: serde_json::Value| -> Result<Vec<serde_json::Value>> {
        match doc {
            serde_json::Value::Array(a) => Ok(a),
            _ => Err(anyhow!("calendar not array")),
        }
    };
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stale_pages = 0u32;
    let mut requests = 0u32;
    for page in 0..CALENDAR_MAX_PAGES {
        let nonce = chrono::Utc::now().timestamp_millis().to_string();
        let page_str = page.to_string();
        requests += 1;
        let got = session.get(
            &format!("{label} page {page}"),
            &url,
            &[
                ("dates", dates),
                ("user-fields", "id"),
                ("nonce", &nonce),
                ("pageSize", &page_size),
                ("page", &page_str),
            ],
            true,
        );
        let events = match got {
            Ok(doc) => as_array(doc)?,
            Err(e) => {
                let msg = e.to_string();
                let fatal = msg == "sling-401" || msg == "sling-429" || msg == "sling-1010"
                    || msg.starts_with("sling-network");
                if page > 0 || fatal {
                    return Err(e);
                }
                // Paging params rejected: fall back to the plain request.
                crate::logging::write_line(
                    "sling",
                    &format!("{label}: paged request failed ({msg}); retrying without pageSize/page"),
                );
                let nonce = chrono::Utc::now().timestamp_millis().to_string();
                let doc = session.get(
                    &format!("{label} unpaged"),
                    &url,
                    &[("dates", dates), ("user-fields", "id"), ("nonce", &nonce)],
                    true,
                )?;
                return as_array(doc);
            }
        };
        let page_len = events.len();
        let mut fresh = 0usize;
        for e in events {
            if seen.insert(event_key(&e)) {
                out.push(e);
                fresh += 1;
            }
        }
        if page_len == 0 || page_len > CALENDAR_PAGE_SIZE {
            break;
        }
        if fresh == 0 {
            stale_pages += 1;
            if stale_pages >= 2 {
                break;
            }
        } else {
            stale_pages = 0;
        }
    }
    crate::logging::write_line("sling", &format!("{label}: {} events over {requests} request(s)", out.len()));
    Ok(out)
}

// ============================================================
// Recurring availability sets (GET /availability?userId=…)
// ============================================================

/// What the per-teacher availability-set fetch produced.
#[derive(Debug, Default, Serialize)]
pub struct AvailabilityFetch {
    /// (teacher id, raw sets Sling returned for them). A teacher with no
    /// sets is listed with an empty vec — that still replaces stored sets.
    pub by_user: Vec<(i64, Vec<serde_json::Value>)>,
    /// Teachers whose sets could not be fetched: (id, reason). Their
    /// previously stored sets are kept.
    pub failed: Vec<(i64, String)>,
    /// Which URL form answered: "org-prefixed" | "bare".
    pub path_form: Option<String>,
}

/// The two URL forms the availability endpoint may live under. Sling's spec
/// documents the bare `/availability`; the calendar and shift endpoints this
/// app already uses take an org prefix (`/{org}/…`), as the legacy extractor
/// did for availability. Both are tried; whichever answers is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AvailPath {
    OrgPrefixed,
    Bare,
}

impl AvailPath {
    fn url(self, cfg: &StudioConfig) -> String {
        match self {
            AvailPath::OrgPrefixed => format!("{BASE_URL}/{}/availability", cfg.org_id),
            AvailPath::Bare => format!("{BASE_URL}/availability"),
        }
    }
    fn name(self) -> &'static str {
        match self {
            AvailPath::OrgPrefixed => "org-prefixed",
            AvailPath::Bare => "bare",
        }
    }
    fn other(self) -> Self {
        match self {
            AvailPath::OrgPrefixed => AvailPath::Bare,
            AvailPath::Bare => AvailPath::OrgPrefixed,
        }
    }
}

/// "This URL form doesn't exist" — try the other one.
fn is_wrong_path(e: &anyhow::Error) -> bool {
    let m = e.to_string();
    m.starts_with("sling-404") || m.starts_with("sling-405")
}

/// Errors that end the whole pull rather than one teacher's fetch.
fn is_fatal(e: &anyhow::Error) -> bool {
    let m = e.to_string();
    m == "sling-401" || m == "sling-1010" || m.starts_with("sling-network")
}

/// Fetch every listed teacher's recurring availability sets. A failure for
/// one teacher is recorded and the rest continue; an expired token (401), a
/// Cloudflare block or a network failure aborts. If the first two teachers
/// fail on both URL forms the endpoint is treated as unreachable and the rest
/// are not attempted (each attempt costs rate limit).
pub fn fetch_availability_sets(
    session: &mut PullSession<'_>,
    cfg: &StudioConfig,
    user_ids: &[i64],
) -> Result<AvailabilityFetch> {
    let mut out = AvailabilityFetch::default();
    let mut chosen: Option<AvailPath> = None;
    for (idx, &uid) in user_ids.iter().enumerate() {
        if chosen.is_none() && idx >= 2 && out.by_user.is_empty() {
            out.failed.push((uid, "availability endpoint unreachable — skipped".to_string()));
            continue;
        }
        let uid_str = uid.to_string();
        let first = chosen.unwrap_or(AvailPath::OrgPrefixed);
        let attempt = |session: &mut PullSession<'_>, form: AvailPath| {
            session.get(
                &format!("availability user {uid} ({})", form.name()),
                &form.url(cfg),
                &[("userId", &uid_str)],
                true,
            )
        };
        let mut result = attempt(session, first).map(|doc| (first, doc));
        if let Err(e) = &result {
            if is_fatal(e) {
                return Err(anyhow!("{e}"));
            }
            if is_wrong_path(e) {
                let second = first.other();
                result = attempt(session, second).map(|doc| (second, doc));
                if let Err(e2) = &result {
                    if is_fatal(e2) {
                        return Err(anyhow!("{e2}"));
                    }
                }
            }
        }
        match result {
            Ok((form, doc)) => match crate::availability::unwrap_sets(&doc) {
                Some(sets) => {
                    if chosen != Some(form) {
                        crate::logging::write_line(
                            "sling",
                            &format!("availability sets: the {} URL form answered ({})", form.name(), form.url(cfg)),
                        );
                        chosen = Some(form);
                    }
                    out.by_user.push((uid, sets));
                }
                None => out.failed.push((uid, "unexpected response shape".to_string())),
            },
            Err(e) => out.failed.push((uid, e.to_string())),
        }
    }
    out.path_form = chosen.map(|f| f.name().to_string());
    crate::logging::write_line(
        "sling",
        &format!(
            "availability sets: {} teacher(s) fetched, {} failed, {} set(s)",
            out.by_user.len(),
            out.failed.len(),
            out.by_user.iter().map(|(_, s)| s.len()).sum::<usize>(),
        ),
    );
    Ok(out)
}

/// The Sling users whose availability sets are worth fetching: active, at the
/// home location, and in at least one position group that isn't switched off
/// in the app — the same people `is_schedulable_teacher` puts on the roster.
pub fn availability_user_ids(
    users: &[SlingUser],
    groups: &[SlingGroup],
    cfg: &StudioConfig,
    inactive_position_ids: &std::collections::HashSet<i64>,
) -> Vec<i64> {
    let schedulable: std::collections::HashSet<i64> = groups
        .iter()
        .filter(|g| g.kind == "position" && !inactive_position_ids.contains(&g.id))
        .map(|g| g.id)
        .collect();
    let mut ids: Vec<i64> = users
        .iter()
        .filter(|u| is_schedulable_teacher(u, cfg.home_location_id, &schedulable))
        .map(|u| u.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// GET /users/concise + /groups on a session (roster for a pull/refresh).
pub fn fetch_roster_in(session: &mut PullSession<'_>) -> Result<(Vec<SlingUser>, Vec<SlingGroup>)> {
    let users_doc = session.get("users", &format!("{BASE_URL}/users/concise"), &[], false)?;
    // Hard-error on a missing/malformed users array rather than returning an
    // empty roster — an empty list would make sync_roster deactivate every
    // teacher.
    let users: Vec<SlingUser> = users_doc.get("users")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("users array missing"))?
        .iter()
        .filter_map(|u| serde_json::from_value(u.clone()).ok())
        .collect();
    let groups_doc = session.get("groups", &format!("{BASE_URL}/groups"), &[], false)?;
    let groups: Vec<SlingGroup> = groups_doc.as_array()
        .ok_or_else(|| anyhow!("groups not array"))?
        .iter()
        .filter_map(|g| serde_json::from_value(g.clone()).ok())
        .collect();
    Ok((users, groups))
}

/// The studio's timezone. Every offset this app sends to Sling is derived
/// from it — never a fixed "-05:00", which is only right for CDT.
pub const STUDIO_TZ: chrono_tz::Tz = chrono_tz::America::Chicago;

/// Resolve a studio-local wall time to an aware datetime. Ambiguous times
/// (the repeated 1 AM hour at fall-back) take the earlier (CDT) instant;
/// nonexistent ones (the skipped 2 AM hour at spring-forward) are shifted
/// forward an hour. Neither occurs for midnight / 23:59:59 boundaries.
pub fn studio_local(ndt: chrono::NaiveDateTime) -> chrono::DateTime<chrono_tz::Tz> {
    use chrono::TimeZone;
    match STUDIO_TZ.from_local_datetime(&ndt) {
        chrono::LocalResult::Single(dt) => dt,
        chrono::LocalResult::Ambiguous(early, _) => early,
        chrono::LocalResult::None => {
            let shifted = ndt + chrono::Duration::hours(1);
            STUDIO_TZ.from_local_datetime(&shifted).earliest()
                .unwrap_or_else(|| STUDIO_TZ.from_utc_datetime(&ndt))
        }
    }
}

/// "YYYY-MM-DDTHH:MM:SS-05:00" (colon offset) for a studio-local wall time,
/// using that instant's own offset (-05:00 CDT / -06:00 CST).
pub fn studio_iso(ndt: chrono::NaiveDateTime) -> String {
    studio_local(ndt).format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// The studio's current month, "YYYY-MM", at instant `now`. Month
/// boundaries follow studio time, not UTC (7pm Central on the 31st is
/// already next month in UTC). Pass `chrono::Utc::now()` in the app.
pub fn studio_month_at(now: chrono::DateTime<chrono::Utc>) -> String {
    now.with_timezone(&STUDIO_TZ).format("%Y-%m").to_string()
}

/// A `db::utc_iso!` string ("2026-11-02T11:00:00Z") re-expressed as
/// studio-local ISO with its own offset ("2026-11-02T05:00:00-06:00"), for
/// payloads that sit next to studio-local shift times (the Claude editor).
/// Anything unparseable is returned unchanged.
pub fn utc_iso_to_studio(utc: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(utc)
        .map(|dt| dt.with_timezone(&STUDIO_TZ).format("%Y-%m-%dT%H:%M:%S%:z").to_string())
        .unwrap_or_else(|_| utc.to_string())
}

/// studio_iso for a DB-style date ("YYYY-MM-DD") + "HH:MM" pair — the shape
/// external_sling_shifts stores. None if either part fails to parse.
pub fn studio_iso_hm(date: &str, hhmm: &str) -> Option<String> {
    let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let t = chrono::NaiveTime::parse_from_str(hhmm, "%H:%M").ok()?;
    Some(studio_iso(d.and_time(t)))
}

/// Offset-bearing dtstart/dtend for a stored shift (date + "HH:MM"), as fed
/// to propose.py. Falls back to the unparsed naive form if the stored values
/// are malformed (never expected: they come from DATE + split_dt).
pub fn shift_iso(date: &str, hhmm: &str) -> String {
    studio_iso_hm(date, hhmm).unwrap_or_else(|| format!("{date}T{hhmm}:00"))
}

fn parse_month(target_month: &str) -> Result<(chrono::NaiveDate, chrono::NaiveDate)> {
    let parts: Vec<&str> = target_month.split('-').collect();
    if parts.len() != 2 { return Err(anyhow!("bad target_month: {target_month}")); }
    let year: i32 = parts[0].parse()?;
    let month: u32 = parts[1].parse()?;
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| anyhow!("invalid date"))?;
    let next_first = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    }.ok_or_else(|| anyhow!("invalid date"))?;
    Ok((first, next_first))
}

/// Returns (startISO, endISO) for the target month: the 1st at 00:00 through
/// the last day at 23:59:59, studio time. Each boundary carries its OWN
/// offset — November 2026 is "2026-11-01T00:00:00-05:00" (still CDT) to
/// "2026-11-30T23:59:59-06:00" (CST). Sling returns empty on historical
/// /calendar queries when the offset is omitted (scripts/legacy/sling_extract.py).
pub fn month_range(target_month: &str) -> Result<(String, String)> {
    let (start, next) = parse_month(target_month)?;
    let end = next.pred_opt().unwrap();
    let midnight = chrono::NaiveTime::MIN;
    let last_sec = chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap();
    Ok((studio_iso(start.and_time(midnight)), studio_iso(end.and_time(last_sec))))
}

/// POST viewdates/cachedates windows for the target month. These are
/// cache-invalidation hints Sling's server uses; we reproduce the web
/// client's padding (prev day .. first-of-next-month + 4 days, cachedates
/// one day wider each side). NB: offset is "-0500" (no colon) here, unlike
/// the calendar `dates=` param which uses "-05:00". Each date uses its own
/// studio offset ("-0600" in CST). Matches scripts/legacy/push_to_sling.py
/// VIEWDATES/CACHEDATES for June 2026.
pub fn view_cache_dates(month: &str) -> Result<(String, String)> {
    let (first, next_first) = parse_month(month)?;
    // NB: "%z" = "-0500" (no colon) — Sling's viewdates/cachedates format. Do
    // NOT change to "%:z"; that colon-form is only for the calendar dates= param.
    let fmt = |d: chrono::NaiveDate| {
        studio_local(d.and_time(chrono::NaiveTime::MIN)).format("%Y-%m-%dT%H:%M:%S%z").to_string()
    };
    let view_start = first - chrono::Duration::days(1);
    let view_end = next_first + chrono::Duration::days(4);
    let cache_start = view_start - chrono::Duration::days(1);
    let cache_end = view_end + chrono::Duration::days(1);
    Ok((
        format!("{}/{}", fmt(view_start), fmt(view_end)),
        format!("{}/{}", fmt(cache_start), fmt(cache_end)),
    ))
}

/// Start of the trailing-history window: the 1st of the month three months
/// before `target_month`, 00:00 studio time, with that date's own offset.
pub fn history_start_iso(target_month: &str) -> Result<String> {
    let (first, _) = parse_month(target_month)?;
    let hist = first
        .checked_sub_months(chrono::Months::new(3))
        .ok_or_else(|| anyhow!("invalid date"))?;
    Ok(studio_iso(hist.and_time(chrono::NaiveTime::MIN)))
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
/// Only planning + published shifts count (matches legacy push_to_sling.py).
pub fn existing_fingerprints(events: &[CalendarEvent], home_location_id: i64) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for ev in events {
        if ev.kind != "shift" { continue; }
        // Unlike filter_events (which lets location-less events through), we
        // require an explicit home-location match here — matches
        // legacy push_to_sling.py's existing_shifts_at_home. A shift returned without
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
                });
            }
        } else {
            let uid = inp.user_id.ok_or_else(|| format!(
                "shift on {} {} ({}) has no teacher assigned — resolve it before pushing",
                inp.date, inp.start, inp.class_name))?;
            specs.push(PushSpec {
                proposal_shift_id: inp.proposal_shift_id, date: inp.date.clone(), start: inp.start.clone(),
                end: inp.end.clone(), position_id: inp.position_id, user_id: uid,
            });
        }
    }
    Ok(specs)
}

/// One month's full pull: roster, groups, the month's calendar (paged), the
/// trailing three months of shifts (paged) and every roster teacher's
/// recurring availability sets. All GETs go through `session` (paced,
/// audited). `inactive_position_ids` are class types switched off in the app.
pub fn pull_month(
    session: &mut PullSession<'_>,
    target_month: &str,
    cfg: &StudioConfig,
    inactive_position_ids: &std::collections::HashSet<i64>,
) -> Result<PullPayload> {
    let (start, end) = month_range(target_month)?;
    let (users, groups) = fetch_roster_in(session)?;

    let dates_param = format!("{start}/{end}");
    let cal_raw = fetch_calendar_paged(session, cfg, &dates_param, &format!("calendar {target_month}"))?;
    let (month_events, month_events_unparsed) = parse_events(&cal_raw);
    crate::logging::write_line(
        "sling",
        &format!(
            "month /calendar {dates_param}: {} raw events, {month_events_unparsed} unparseable",
            cal_raw.len()
        ),
    );

    // Offset matters: without it Sling returns empty for historical
    // /calendar queries (scripts/legacy/sling_extract.py).
    let hist_dates_param = format!("{}/{start}", history_start_iso(target_month)?);
    let hist_raw = fetch_calendar_paged(session, cfg, &hist_dates_param, &format!("history before {target_month}"))?;
    let (parsed, _) = parse_events(&hist_raw);
    let parsed_count = parsed.len();
    let history_shifts: Vec<CalendarEvent> = parsed
        .into_iter()
        .filter(|e: &CalendarEvent|
            e.kind == "shift"
            && e.location.as_ref().is_none_or(|l| l.id == cfg.home_location_id)
        )
        .collect();
    // The kinds distribution spots field-name drift.
    let mut kind_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for e in &hist_raw {
        let k = e.get("type").and_then(|v| v.as_str()).unwrap_or("<missing type>").to_string();
        *kind_counts.entry(k).or_insert(0) += 1;
    }
    crate::logging::write_line(
        "sling",
        &format!(
            "history /calendar {hist_dates_param}: {} raw, {parsed_count} parsed, {} after shift+home location filter; types {kind_counts:?}",
            hist_raw.len(),
            history_shifts.len()
        ),
    );

    let avail_users = availability_user_ids(&users, &groups, cfg, inactive_position_ids);
    let availability = fetch_availability_sets(session, cfg, &avail_users)?;

    Ok(PullPayload {
        target_month: target_month.to_string(),
        users,
        groups,
        month_events,
        history_shifts,
        availability,
        month_events_unparsed,
    })
}

/// The location options to offer for "home location": the location groups the
/// user belongs to, sorted by name. If the user belongs to none, fall back to
/// every location group in the org so the dropdown is never empty.
pub fn select_locations(
    group_ids: &[i64],
    loc_names: &std::collections::HashMap<i64, String>,
) -> Vec<DiscoveredLocation> {
    let mut mine: Vec<DiscoveredLocation> = group_ids.iter()
        .filter_map(|g| loc_names.get(g).map(|n| DiscoveredLocation { id: *g, name: n.clone() }))
        .collect();
    if mine.is_empty() {
        mine = loc_names.iter().map(|(id, n)| DiscoveredLocation { id: *id, name: n.clone() }).collect();
    }
    mine.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    mine
}

/// Discover the studio's org / acting-user / location options from Sling using
/// a freshly captured token. `org_hint` is the org id parsed from the login
/// request URL (guaranteed when present). Best-effort: returns an error only if
/// it can't determine the org at all.
pub fn discover_studio(token: &str, org_hint: Option<i64>) -> Result<DiscoveredStudio> {
    // account/session may be org-scoped; use the hint's org when we have it.
    let session = if let Some(org) = org_hint {
        http_get(token, &format!("{BASE_URL}/{org}/account/session"))?
    } else {
        http_get(token, &format!("{BASE_URL}/account/session"))?
    };
    let (acting_user_id, acting_user_name, org_from_session) = parse_session(&session)?;
    let org_id = org_hint.or(org_from_session)
        .ok_or_else(|| anyhow!("couldn't determine Sling org — enter it manually"))?;

    // The acting user's location memberships come from their group_ids.
    let users_doc = http_get(token, &format!("{BASE_URL}/users/concise"))?;
    let group_ids: Vec<i64> = users_doc.get("users")
        .and_then(|v| v.as_array())
        .into_iter().flatten()
        .filter_map(|u| serde_json::from_value::<SlingUser>(u.clone()).ok())
        .find(|u| u.id == acting_user_id)
        .map(|u| u.group_ids)
        .unwrap_or_default();

    let groups_doc = http_get(token, &format!("{BASE_URL}/groups"))?;
    let groups: Vec<SlingGroup> = groups_doc.as_array()
        .ok_or_else(|| anyhow!("groups not array"))?
        .iter().filter_map(|g| serde_json::from_value(g.clone()).ok()).collect();
    let loc_names = location_name_by_id(&groups);

    let org_name = session_org_name(&session, org_id);
    Ok(DiscoveredStudio {
        org_id, acting_user_id, acting_user_name, org_name,
        locations: select_locations(&group_ids, &loc_names),
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

/// A Sling user belongs in the roster iff they are active, a member of the
/// home-location group, and qualified for at least one schedulable position.
pub fn is_schedulable_teacher(
    user: &SlingUser,
    home_location_id: i64,
    schedulable_position_ids: &std::collections::HashSet<i64>,
) -> bool {
    user.active
        && user.group_ids.contains(&home_location_id)
        && user.group_ids.iter().any(|g| schedulable_position_ids.contains(g))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Group IDs of position-type groups. The live pull derives
    /// qualifications elsewhere; this only pins the fixture's group shape.
    fn position_group_ids(groups: &[SlingGroup]) -> std::collections::HashSet<i64> {
        groups
            .iter()
            .filter(|g| g.kind == "position")
            .map(|g| g.id)
            .collect()
    }

    #[test]
    fn studio_month_follows_central_time_not_utc() {
        let utc = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&chrono::Utc);
        // 7:30pm CDT on Sept 30 is already October in UTC.
        assert_eq!(studio_month_at(utc("2026-10-01T00:30:00Z")), "2026-09");
        assert_eq!(studio_month_at(utc("2026-10-01T05:00:00Z")), "2026-10");
        // CST: New Year's Eve 11pm Central = 05:00Z Jan 1.
        assert_eq!(studio_month_at(utc("2027-01-01T05:00:00Z")), "2026-12");
        assert_eq!(studio_month_at(utc("2027-01-01T06:00:00Z")), "2027-01");
    }

    #[test]
    fn utc_iso_to_studio_uses_the_instants_own_offset() {
        assert_eq!(utc_iso_to_studio("2026-11-02T11:00:00Z"), "2026-11-02T05:00:00-06:00");
        assert_eq!(utc_iso_to_studio("2026-10-31T10:45:00Z"), "2026-10-31T05:45:00-05:00");
        assert_eq!(utc_iso_to_studio("garbage"), "garbage");
    }

    #[test]
    fn month_range_returns_correct_bounds() {
        let (s, e) = month_range("2026-06").unwrap();
        assert_eq!(s, "2026-06-01T00:00:00-05:00");
        assert_eq!(e, "2026-06-30T23:59:59-05:00");
        let (s2, e2) = month_range("2026-12").unwrap();
        assert_eq!(s2, "2026-12-01T00:00:00-06:00");
        assert_eq!(e2, "2026-12-31T23:59:59-06:00");
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
        // Matches the constants the legacy push_to_sling.py used for June 2026.
        assert_eq!(view, "2026-05-31T00:00:00-0500/2026-07-05T00:00:00-0500");
        assert_eq!(cache, "2026-05-30T00:00:00-0500/2026-07-06T00:00:00-0500");
    }

    #[test]
    fn view_cache_dates_handles_december_year_rollover() {
        let (view, cache) = view_cache_dates("2026-12").unwrap();
        assert_eq!(view, "2026-11-30T00:00:00-0600/2027-01-05T00:00:00-0600");
        assert_eq!(cache, "2026-11-29T00:00:00-0600/2027-01-06T00:00:00-0600");
    }

    // --- DST boundaries (America/Chicago). 2026: CDT ends Sun Nov 1 02:00.
    // 2027: CDT starts Sun Mar 14 02:00. ---

    #[test]
    fn month_range_october_2026_is_all_cdt() {
        let (s, e) = month_range("2026-10").unwrap();
        assert_eq!(s, "2026-10-01T00:00:00-05:00");
        assert_eq!(e, "2026-10-31T23:59:59-05:00");
    }

    #[test]
    fn month_range_november_2026_uses_each_boundarys_offset() {
        let (s, e) = month_range("2026-11").unwrap();
        // Nov 1 midnight is still CDT (fall-back is at 02:00 that morning).
        assert_eq!(s, "2026-11-01T00:00:00-05:00");
        assert_eq!(e, "2026-11-30T23:59:59-06:00");
    }

    #[test]
    fn month_range_march_2027_spans_spring_forward() {
        let (s, e) = month_range("2027-03").unwrap();
        assert_eq!(s, "2027-03-01T00:00:00-06:00");
        assert_eq!(e, "2027-03-31T23:59:59-05:00");
        let (s, e) = month_range("2027-02").unwrap();
        assert_eq!(s, "2027-02-01T00:00:00-06:00");
        assert_eq!(e, "2027-02-28T23:59:59-06:00");
    }

    #[test]
    fn view_cache_dates_across_fall_back() {
        // October window pads into November: Oct 30/Nov 5 straddle Nov 1.
        let (view, cache) = view_cache_dates("2026-10").unwrap();
        assert_eq!(view, "2026-09-30T00:00:00-0500/2026-11-05T00:00:00-0600");
        assert_eq!(cache, "2026-09-29T00:00:00-0500/2026-11-06T00:00:00-0600");
        let (view, _) = view_cache_dates("2026-11").unwrap();
        assert_eq!(view, "2026-10-31T00:00:00-0500/2026-12-05T00:00:00-0600");
    }

    #[test]
    fn view_cache_dates_across_spring_forward() {
        let (view, cache) = view_cache_dates("2027-03").unwrap();
        assert_eq!(view, "2027-02-28T00:00:00-0600/2027-04-05T00:00:00-0500");
        assert_eq!(cache, "2027-02-27T00:00:00-0600/2027-04-06T00:00:00-0500");
    }

    #[test]
    fn history_start_uses_its_own_offset() {
        assert_eq!(history_start_iso("2026-11").unwrap(), "2026-08-01T00:00:00-05:00");
        assert_eq!(history_start_iso("2027-03").unwrap(), "2026-12-01T00:00:00-06:00");
        assert_eq!(history_start_iso("2027-02").unwrap(), "2026-11-01T00:00:00-05:00");
        assert_eq!(history_start_iso("2026-01").unwrap(), "2025-10-01T00:00:00-05:00");
    }

    #[test]
    fn studio_iso_hm_handles_transition_days_and_edge_hours() {
        // Class times on the transition days themselves.
        assert_eq!(studio_iso_hm("2026-11-01", "09:00").unwrap(), "2026-11-01T09:00:00-06:00");
        assert_eq!(studio_iso_hm("2026-10-31", "09:00").unwrap(), "2026-10-31T09:00:00-05:00");
        assert_eq!(studio_iso_hm("2027-03-14", "05:45").unwrap(), "2027-03-14T05:45:00-05:00");
        assert_eq!(studio_iso_hm("2027-03-13", "05:45").unwrap(), "2027-03-13T05:45:00-06:00");
        // Ambiguous 01:30 on fall-back day resolves to the earlier (CDT) instant.
        assert_eq!(studio_iso_hm("2026-11-01", "01:30").unwrap(), "2026-11-01T01:30:00-05:00");
        // Nonexistent 02:30 on spring-forward day is shifted to 03:30 CDT.
        assert_eq!(studio_iso_hm("2027-03-14", "02:30").unwrap(), "2027-03-14T03:30:00-05:00");
        assert!(studio_iso_hm("2026-13-01", "09:00").is_none());
        assert!(studio_iso_hm("2026-11-01", "9am").is_none());
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
                end: "06:45".into(), position_id: 29470407, user_id: Some(1001),
                class_name: "Empower".into(), is_coteach: false, coteach_label: None, is_dropped: false },
            ProposalShiftInput { proposal_shift_id: 11, date: "2026-06-02".into(), start: "09:00".into(),
                end: "10:00".into(), position_id: 29303965, user_id: Some(1001),
                class_name: "Classic".into(), is_coteach: true, coteach_label: Some("Teacher A + Teacher E".into()), is_dropped: false },
            ProposalShiftInput { proposal_shift_id: 12, date: "2026-06-03".into(), start: "09:00".into(),
                end: "10:00".into(), position_id: 29303965, user_id: None,
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
            end: "06:45".into(), position_id: 29470407, user_id: None,
            class_name: "Empower".into(), is_coteach: false, coteach_label: None, is_dropped: false }];
        let e = build_push_specs(&inputs, &name_to_id).unwrap_err();
        assert!(e.contains("no teacher"), "got: {e}");
    }

    #[test]
    fn build_push_specs_errors_on_unknown_coteach_name() {
        let mut name_to_id = std::collections::HashMap::new();
        name_to_id.insert("Teacher A".to_string(), 1001i64);
        let inputs = vec![ProposalShiftInput { proposal_shift_id: 30, date: "2026-06-02".into(), start: "09:00".into(),
            end: "10:00".into(), position_id: 29303965, user_id: Some(1001),
            class_name: "Classic".into(), is_coteach: true, coteach_label: Some("Teacher A + Ghost".into()), is_dropped: false }];
        let e = build_push_specs(&inputs, &name_to_id).unwrap_err();
        assert!(e.contains("Ghost"), "got: {e}");
    }

    #[test]
    fn parse_session_reads_user_and_optional_org() {
        let v = serde_json::json!({ "user": { "id": 29470393, "name": "Lead Teacher" } });
        let (uid, name, org) = parse_session(&v).unwrap();
        assert_eq!(uid, 29470393);
        assert_eq!(name, "Lead Teacher");
        assert_eq!(org, None);

        let v2 = serde_json::json!({ "org": { "id": "1193381" }, "user": { "id": "42", "name": "X" } });
        let (uid2, _n2, org2) = parse_session(&v2).unwrap();
        assert_eq!(uid2, 42);
        assert_eq!(org2, Some(1193381));
    }

    #[test]
    fn session_org_name_requires_matching_id() {
        let v = serde_json::json!({ "org": { "id": "77", "name": "Barre Studio" }, "user": { "id": 1 } });
        assert_eq!(session_org_name(&v, 77), "Barre Studio");
        assert_eq!(session_org_name(&v, 78), "");
        assert_eq!(session_org_name(&serde_json::json!({}), 77), "");
    }

    #[test]
    fn parse_session_errors_without_user() {
        let v = serde_json::json!({ "nope": true });
        assert!(parse_session(&v).is_err());
    }

    #[test]
    fn select_locations_intersects_then_falls_back_to_all() {
        let mut names = std::collections::HashMap::new();
        names.insert(5i64, "Pinnacle".to_string());
        names.insert(7i64, "Downtown".to_string());
        names.insert(9i64, "Westside".to_string());
        // user belongs to 5 and 7 (and a non-location group 100)
        let got = select_locations(&[5, 100, 7], &names);
        assert_eq!(got.iter().map(|l| l.id).collect::<Vec<_>>(), vec![7, 5]); // sorted by name: Downtown, Pinnacle
        // user belongs to no location group -> fall back to ALL locations, sorted by name
        let none = select_locations(&[100, 200], &names);
        assert_eq!(none.len(), 3);
        assert_eq!(none[0].name, "Downtown");
    }

    #[test]
    fn is_schedulable_teacher_requires_active_home_and_qualified() {
        let mut schedulable = std::collections::HashSet::new();
        schedulable.insert(900i64);
        let home = 5i64;
        let mk = |active: bool, groups: Vec<i64>| SlingUser {
            id: 1, name: "T".into(), lastname: "X".into(), active, group_ids: groups,
        };
        assert!(is_schedulable_teacher(&mk(true, vec![5, 900]), home, &schedulable));
        assert!(!is_schedulable_teacher(&mk(false, vec![5, 900]), home, &schedulable));
        assert!(!is_schedulable_teacher(&mk(true, vec![900]), home, &schedulable));
        assert!(!is_schedulable_teacher(&mk(true, vec![5, 777]), home, &schedulable));
    }

    #[test]
    fn build_shift_body_matches_sling_contract() {
        let s = PushSpec { proposal_shift_id: 1, date: "2026-06-01".into(), start: "05:45".into(),
            end: "06:45".into(), position_id: 29470407, user_id: 1001 };
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

    // --- pull session, calendar paging, availability sets (no network) ---

    fn test_cfg() -> StudioConfig {
        StudioConfig { org_id: 41822, acting_user_id: 1930001, home_location_id: 901 }
    }

    fn q<'a>(query: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
        query.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    fn events(range: std::ops::Range<usize>) -> serde_json::Value {
        serde_json::Value::Array(
            range
                .map(|i| serde_json::json!({"id": i.to_string(), "type": "shift",
                    "dtstart": "2026-11-02T05:45:00-06:00", "dtend": "2026-11-02T06:45:00-06:00"}))
                .collect(),
        )
    }

    #[test]
    fn availability_event_with_non_numeric_id_is_kept() {
        // Sling documents availability-event ids as strings; a non-numeric
        // one used to fail the whole event and silently drop the block.
        let raw = vec![
            serde_json::json!({"id": "avail-77:2026-11-03", "type": "availability",
                "dtstart": "2026-11-03T09:45:00-06:00", "dtend": "2026-11-03T10:45:00-06:00",
                "user": {"id": "1930004"}}),
            serde_json::json!({"id": 5, "type": "leave", "dtstart": "a", "dtend": "b", "user": {"id": 7}}),
            serde_json::json!({"type": "shift", "user": {"id": "not a number"}}),
        ];
        let (parsed, dropped) = parse_events(&raw);
        assert_eq!((parsed.len(), dropped), (2, 1));
        assert_eq!(parsed[0].id, None);
        assert_eq!(parsed[0].kind, "availability");
        assert_eq!(event_user_id(&parsed[0]), Some(1930004));
        assert_eq!(parsed[1].id, Some(5));
    }

    #[test]
    fn calendar_paging_follows_zero_based_pages_to_an_empty_page() {
        let mut pages_asked = Vec::new();
        let mut session = PullSession::with(|url, query| {
            assert!(url.ends_with("/41822/calendar/41822/users/1930001"), "{url}");
            assert_eq!(q(query, "pageSize"), Some("500"));
            assert_eq!(q(query, "user-fields"), Some("id"));
            assert!(q(query, "dates").is_some() && q(query, "nonce").is_some());
            let page: usize = q(query, "page").unwrap().parse().unwrap();
            pages_asked.push(page);
            Ok(match page {
                0 => events(0..500),
                1 => events(500..1000),
                2 => events(1000..1120),
                _ => events(0..0),
            })
        });
        let got = fetch_calendar_paged(&mut session, &test_cfg(), "a/b", "calendar test").unwrap();
        assert_eq!(got.len(), 1120);
        assert_eq!(session.audit.len(), 4, "every page is in the audit trail");
        assert_eq!(session.audit[2].response.as_array().unwrap().len(), 120);
        drop(session);
        // A short page is not trusted as the last one (a server-side cap
        // below our page size would look the same): stop on the empty page.
        assert_eq!(pages_asked, [0, 1, 2, 3]);
    }

    #[test]
    fn calendar_paging_handles_one_based_pages() {
        // page=0 and page=1 both return the first page.
        let mut session = PullSession::with(|_, query| {
            let page: usize = q(query, "page").unwrap().parse().unwrap();
            Ok(match page {
                0 | 1 => events(0..500),
                2 => events(500..700),
                _ => events(0..0),
            })
        });
        let got = fetch_calendar_paged(&mut session, &test_cfg(), "a/b", "t").unwrap();
        assert_eq!(got.len(), 700, "no duplicates, nothing truncated");
        assert_eq!(session.calls(), 4);
    }

    #[test]
    fn calendar_paging_stops_when_sling_ignores_the_params() {
        // Everything comes back every time, smaller than a page: two pages
        // that add nothing end the loop.
        let mut session = PullSession::with(|_, _| Ok(events(0..180)));
        assert_eq!(fetch_calendar_paged(&mut session, &test_cfg(), "a/b", "t").unwrap().len(), 180);
        assert_eq!(session.calls(), 3);
        // Larger than a page: paging is evidently ignored — one request.
        let mut session = PullSession::with(|_, _| Ok(events(0..640)));
        assert_eq!(fetch_calendar_paged(&mut session, &test_cfg(), "a/b", "t").unwrap().len(), 640);
        assert_eq!(session.calls(), 1);
        // Events without ids are de-duplicated by content.
        let mut session = PullSession::with(|_, _| Ok(serde_json::json!([{"type": "leave", "dtstart": "x"}])));
        assert_eq!(fetch_calendar_paged(&mut session, &test_cfg(), "a/b", "t").unwrap().len(), 1);
    }

    #[test]
    fn calendar_paging_falls_back_when_params_are_rejected() {
        let mut session = PullSession::with(|_, query| {
            if q(query, "page").is_some() {
                Err(anyhow!("sling-400: unknown parameter"))
            } else {
                Ok(events(0..42))
            }
        });
        assert_eq!(fetch_calendar_paged(&mut session, &test_cfg(), "a/b", "t").unwrap().len(), 42);
        assert_eq!(session.calls(), 2);
        // An expired token is never retried as "unpaged".
        let mut session = PullSession::with(|_, _| Err(anyhow!("sling-401")));
        let e = fetch_calendar_paged(&mut session, &test_cfg(), "a/b", "t").unwrap_err();
        assert_eq!(e.to_string(), "sling-401");
        assert_eq!(session.calls(), 1);
        // A non-array body is an error, not an empty calendar.
        let mut session = PullSession::with(|_, _| Ok(serde_json::json!({"message": "hm"})));
        assert!(fetch_calendar_paged(&mut session, &test_cfg(), "a/b", "t").is_err());
    }

    #[test]
    fn session_retries_rate_limits_and_gives_up_after_three() {
        let mut n = 0;
        let mut session = PullSession::with(|_, _| {
            n += 1;
            if n < 3 { Err(anyhow!("sling-429")) } else { Ok(serde_json::json!([])) }
        });
        assert!(session.get("x", "u", &[], true).is_ok());
        assert_eq!(session.calls(), 3);
        assert_eq!(session.audit.len(), 1, "one audit entry per logical request");
        let mut session = PullSession::with(|_, _| Err(anyhow!("sling-429")));
        assert_eq!(session.get("x", "u", &[], true).unwrap_err().to_string(), "sling-429");
        assert_eq!(session.calls(), 3);
        assert_eq!(session.audit[0].outcome, "sling-429");
        // Unrecorded requests (the roster) leave no audit entry.
        let mut session = PullSession::with(|_, _| Ok(serde_json::json!({"users": []})));
        session.get("users", "u", &[], false).unwrap();
        assert!(session.audit.is_empty());
    }

    fn sets_fixture() -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string("test_fixtures/sling_availability_sets.json").unwrap()).unwrap()
    }

    #[test]
    fn availability_sets_fall_back_to_the_bare_path_and_remember_it() {
        let fx = sets_fixture();
        let mut urls: Vec<String> = Vec::new();
        let mut session = PullSession::with(|url, query| {
            urls.push(format!("{url}?userId={}", q(query, "userId").unwrap()));
            if url.contains("/41822/availability") {
                return Err(anyhow!("sling-404: not found"));
            }
            Ok(match q(query, "userId").unwrap() {
                "1930004" => fx["user_julia"].clone(),
                "1930001" => fx["user_alex_wrapped"].clone(),
                _ => fx["user_kayla_none"].clone(),
            })
        });
        let got = fetch_availability_sets(&mut session, &test_cfg(), &[1930001, 1930002, 1930004]).unwrap();
        assert_eq!(got.path_form.as_deref(), Some("bare"));
        assert!(got.failed.is_empty());
        let counts: Vec<(i64, usize)> = got.by_user.iter().map(|(u, s)| (*u, s.len())).collect();
        assert_eq!(counts, [(1930001, 1), (1930002, 0), (1930004, 4)]);
        assert_eq!(session.audit.len(), 4);
        assert_eq!(session.audit[0].outcome, "sling-404: not found");
        drop(session);
        // The org-prefixed form is tried once; after that only the form
        // that answered.
        let base = "https://api.getsling.com/v1";
        assert_eq!(
            urls,
            [
                format!("{base}/41822/availability?userId=1930001"),
                format!("{base}/availability?userId=1930001"),
                format!("{base}/availability?userId=1930002"),
                format!("{base}/availability?userId=1930004"),
            ]
        );
    }

    #[test]
    fn availability_sets_prefer_the_org_prefixed_path_when_it_answers() {
        let mut session = PullSession::with(|url, _| {
            assert!(url.ends_with("/41822/availability"), "{url}");
            Ok(serde_json::json!([]))
        });
        let got = fetch_availability_sets(&mut session, &test_cfg(), &[1, 2]).unwrap();
        assert_eq!(got.path_form.as_deref(), Some("org-prefixed"));
        assert_eq!(session.calls(), 2);
    }

    #[test]
    fn availability_set_failures_are_per_teacher_but_auth_aborts() {
        // One teacher errors; the rest still arrive.
        let mut session = PullSession::with(|_, query| match q(query, "userId").unwrap() {
            "2" => Err(anyhow!("sling-500: boom")),
            "3" => Ok(serde_json::json!({"unexpected": true})),
            _ => Ok(serde_json::json!([{"interval": 1}])),
        });
        let got = fetch_availability_sets(&mut session, &test_cfg(), &[1, 2, 3, 4]).unwrap();
        assert_eq!(got.by_user.iter().map(|(u, _)| *u).collect::<Vec<_>>(), [1, 4]);
        assert_eq!(got.failed, [(2, "sling-500: boom".to_string()), (3, "unexpected response shape".to_string())]);

        // Neither form exists: stop after two teachers instead of burning
        // the rate limit on all of them.
        let mut session = PullSession::with(|_, _| Err(anyhow!("sling-404: nope")));
        let got = fetch_availability_sets(&mut session, &test_cfg(), &[1, 2, 3, 4, 5]).unwrap();
        assert!(got.by_user.is_empty() && got.path_form.is_none());
        assert_eq!(got.failed.len(), 5);
        assert_eq!(session.calls(), 4);

        // An expired token aborts the pull.
        let mut session = PullSession::with(|_, _| Err(anyhow!("sling-401")));
        let e = fetch_availability_sets(&mut session, &test_cfg(), &[1, 2]).unwrap_err();
        assert_eq!(e.to_string(), "sling-401");
    }

    #[test]
    fn availability_is_fetched_for_roster_teachers_only() {
        let user = |id: i64, active: bool, groups: &[i64]| SlingUser {
            id, name: format!("u{id}"), lastname: String::new(), active, group_ids: groups.to_vec(),
        };
        let groups = vec![
            SlingGroup { id: 101, name: "Classic".into(), kind: "position".into() },
            SlingGroup { id: 102, name: "Sales Rep".into(), kind: "position".into() },
            SlingGroup { id: 901, name: "Home".into(), kind: "location".into() },
            SlingGroup { id: 902, name: "Other".into(), kind: "location".into() },
        ];
        let users = vec![
            user(5, true, &[101, 901]),
            user(1, true, &[101, 102, 901]),
            user(2, false, &[101, 901]), // inactive
            user(3, true, &[101, 902]),  // other location
            user(4, true, &[102, 901]),  // only a switched-off position
            user(6, true, &[901]),       // no position
        ];
        let inactive: std::collections::HashSet<i64> = [102].into_iter().collect();
        assert_eq!(availability_user_ids(&users, &groups, &test_cfg(), &inactive), [1, 5]);
    }

    #[test]
    fn pull_month_collects_calendar_history_and_sets() {
        let fx = sets_fixture();
        let mut session = PullSession::with(|url, query| {
            if url.ends_with("/users/concise") {
                return Ok(serde_json::json!({"users": [
                    {"id": 1930004, "name": "Julia", "lastname": "Stone", "active": true, "groupIds": [101, 901]}]}));
            }
            if url.ends_with("/groups") {
                return Ok(serde_json::json!([
                    {"id": 101, "name": "Classic", "type": "position"},
                    {"id": 901, "name": "Home", "type": "location"}]));
            }
            if url.contains("/calendar/") {
                if q(query, "page") != Some("0") {
                    return Ok(serde_json::json!([]));
                }
                let history = q(query, "dates").unwrap().starts_with("2026-08-01");
                return Ok(if history {
                    serde_json::json!([{"id": "1", "type": "shift", "dtstart": "2026-10-06T09:45:00-05:00",
                        "dtend": "2026-10-06T10:45:00-05:00", "user": {"id": 1930004},
                        "position": {"id": 101}, "location": {"id": 901}, "status": "published"}])
                } else {
                    serde_json::json!([
                        {"id": "x-1", "type": "availability", "dtstart": "2026-11-05T08:00:00-06:00",
                         "dtend": "2026-11-05T09:00:00-06:00", "user": {"id": 1930004}},
                        {"id": {"nested": true}, "type": "leave"}])
                });
            }
            assert!(url.contains("/availability"), "{url}");
            Ok(fx["user_julia"].clone())
        });
        let none = std::collections::HashSet::new();
        let p = pull_month(&mut session, "2026-11", &test_cfg(), &none).unwrap();
        assert_eq!(p.users.len(), 1);
        assert_eq!(p.month_events.len(), 1);
        assert_eq!(p.month_events_unparsed, 1);
        assert_eq!(p.history_shifts.len(), 1);
        assert_eq!(p.availability.by_user.len(), 1);
        assert_eq!(p.availability.by_user[0].1.len(), 4);
        // Audit: calendar pages + availability responses, never the roster.
        assert!(session.audit.iter().all(|a| !a.url.contains("users/concise") && !a.url.ends_with("/groups")));
        assert!(session.audit.iter().any(|a| a.label.starts_with("calendar 2026-11 page 0")));
        assert!(session.audit.iter().any(|a| a.label.starts_with("availability user 1930004")));
    }
}
