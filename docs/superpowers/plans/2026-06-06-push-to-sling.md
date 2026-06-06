# Push to Sling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the missing Push step so the lead teacher can push an approved proposal to Sling as planning-status shifts, with a dry-run preview, live batched progress, idempotent re-push, and a full audit trail.

**Architecture:** A pure-Rust port of `scripts/push_to_sling.py`. Low-level HTTP primitives + pure helpers (spec-building, co-teach expansion, dedupe fingerprints, request-body shape, `viewdates`/`cachedates`) live in `src-tauri/src/sling.rs` alongside the existing pull code. Orchestration (build specs from DuckDB, dedupe against Sling, batch-POST with throttle/backoff, write `pushes`/`push_results`, emit progress events) lives in two Tauri commands in `commands.rs`, mirroring how `pull_month_from_sling` and `review_proposal` already work. The frontend adds a `PushModal` opened from the proposal detail toolbar.

**Tech Stack:** Rust (ureq, serde_json, duckdb, chrono, Tauri 2 events), React + TypeScript, plain CSS.

**Spec:** `docs/superpowers/specs/2026-06-06-push-to-sling-design.md`

---

## File structure

- **`src-tauri/src/sling.rs`** (modify) — add push primitives + pure helpers + unit tests. Reuses existing `StudioConfig`, `CalendarEvent`, flex deserializers, `month_range`, and the browser-header set.
- **`src-tauri/src/commands.rs`** (modify) — add `push_proposal_dry_run` and `push_proposal_execute` commands plus a private `build_specs_for_proposal` helper.
- **`src-tauri/src/lib.rs`** (modify) — register the two new commands.
- **`src/types.ts`** (modify) — `PushPreview`, `PushPreviewItem`, `PushSummary`, `PushProgress`.
- **`src/lib/api.ts`** (modify) — `pushProposalDryRun`, `pushProposalExecute`.
- **`src/components/PushModal.tsx`** (create) — preview → confirm → live progress → summary.
- **`src/App.tsx`** (modify) — Push button in the proposal detail toolbar; render `PushModal`; route 401 to the existing `SlingTokenModal`.
- **`src/styles.css`** (modify) — progress-bar + result-row styles (reuses existing `.modal`/`.modal-backdrop`).
- **`docs/architecture.md`** (modify) — update the push step description (in-process Rust, no CSV).

### Type/signature contract (used across tasks — keep names exact)

```rust
// sling.rs
pub struct PushSpec {
    pub proposal_shift_id: i64,
    pub date: String,        // "2026-06-01"
    pub start: String,       // "05:45"
    pub end: String,         // "06:45"
    pub position_id: i64,
    pub user_id: i64,
    pub class_name: String,  // display only
    pub teacher_name: String,// display only
}
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
pub fn view_cache_dates(month: &str) -> anyhow::Result<(String, String)>; // (viewdates, cachedates)
pub fn build_push_specs(inputs: &[ProposalShiftInput], name_to_id: &std::collections::HashMap<String, i64>) -> Result<Vec<PushSpec>, String>;
pub fn split_dt(dt: &str) -> (String, String); // ("2026-06-01","05:45")
pub fn spec_fingerprint(s: &PushSpec, home_location_id: i64) -> String;
pub fn existing_fingerprints(events: &[CalendarEvent], home_location_id: i64) -> std::collections::HashSet<String>;
pub fn build_shift_body(s: &PushSpec, home_location_id: i64) -> serde_json::Value;
pub fn push_shift(token: &str, cfg: &StudioConfig, s: &PushSpec, viewdates: &str, cachedates: &str) -> anyhow::Result<i64>;
pub fn fetch_calendar(token: &str, cfg: &StudioConfig, month: &str) -> anyhow::Result<Vec<CalendarEvent>>;
```

```ts
// types.ts
export interface PushPreviewItem { date: string; start: string; end: string; class_name: string; teacher_name: string; }
export interface PushPreview { total: number; skipped_count: number; to_create: PushPreviewItem[]; }
export interface PushSummary { push_id: number; created: number; failed: number; skipped: number; }
export interface PushProgress { total: number; done: number; created: number; failed: number; skipped: number; last_label: string; last_outcome: string; }
```

---

## Task 1: `view_cache_dates` helper (sling.rs)

Computes the POST `viewdates`/`cachedates` windows from the target month, reproducing the proven June-2026 window the Python script hardcoded. **Offset format is `-0500` (no colon)** — different from the calendar `dates=` param's `-05:00`.

**Files:**
- Modify: `src-tauri/src/sling.rs`
- Test: same file, `#[cfg(test)]` module

- [ ] **Step 1: Write the failing test** (add inside the existing `mod tests` in `sling.rs`)

```rust
#[test]
fn view_cache_dates_reproduce_june_window() {
    let (view, cache) = view_cache_dates("2026-06").unwrap();
    // Matches the constants the working push_to_sling.py used for June 2026.
    assert_eq!(view, "2026-05-31T00:00:00-0500/2026-07-05T00:00:00-0500");
    assert_eq!(cache, "2026-05-30T00:00:00-0500/2026-07-06T00:00:00-0500");
}

#[test]
fn view_cache_dates_handles_december_year_rollover() {
    let (view, _cache) = view_cache_dates("2026-12").unwrap();
    assert_eq!(view, "2026-11-30T00:00:00-0500/2027-01-05T00:00:00-0500");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml view_cache_dates`
Expected: FAIL — `cannot find function view_cache_dates`.

- [ ] **Step 3: Implement** (add near `month_range` in `sling.rs`)

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml view_cache_dates`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/sling.rs
git commit -m "feat(push): view_cache_dates helper reproducing the proven Sling window"
```

---

## Task 2: `split_dt` + dedupe fingerprints (sling.rs)

Pure helpers for matching intended shifts against shifts already in Sling.

**Files:**
- Modify: `src-tauri/src/sling.rs`
- Test: same file

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn split_dt_extracts_date_and_hhmm() {
    assert_eq!(split_dt("2026-06-01T05:45:00-05:00"), ("2026-06-01".into(), "05:45".into()));
    assert_eq!(split_dt("2026-06-01"), ("2026-06-01".into(), "00:00".into()));
}

#[test]
fn existing_fingerprints_filters_and_keys_correctly() {
    let events = vec![
        // home shift, planning -> included
        CalendarEvent { id: Some(1), kind: "shift".into(), dtstart: "2026-06-01T05:45:00-05:00".into(),
            dtend: "2026-06-01T06:45:00-05:00".into(), user: Some(SlingEventUserRef { id: 1001 }),
            users: None, position: Some(SlingEventPositionRef { id: 29470407 }),
            location: Some(SlingEventLocationRef { id: 5 }), status: Some("planning".into()) },
        // wrong location -> excluded
        CalendarEvent { id: Some(2), kind: "shift".into(), dtstart: "2026-06-01T05:45:00-05:00".into(),
            dtend: "2026-06-01T06:45:00-05:00".into(), user: Some(SlingEventUserRef { id: 1002 }),
            users: None, position: Some(SlingEventPositionRef { id: 29470407 }),
            location: Some(SlingEventLocationRef { id: 999 }), status: Some("planning".into()) },
        // leave event -> excluded
        CalendarEvent { id: Some(3), kind: "leave".into(), dtstart: "2026-06-02T00:00:00-05:00".into(),
            dtend: "".into(), user: Some(SlingEventUserRef { id: 1001 }), users: None,
            position: None, location: Some(SlingEventLocationRef { id: 5 }), status: None },
    ];
    let fp = existing_fingerprints(&events, 5);
    assert_eq!(fp.len(), 1);
    assert!(fp.contains("2026-06-01|05:45|1001|29470407|5"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml fingerprint`
Expected: FAIL — `cannot find function split_dt` / `existing_fingerprints`.

- [ ] **Step 3: Implement** (add to `sling.rs`)

```rust
use std::collections::HashSet;

/// Split a Sling dtstart ("2026-06-01T05:45:00-05:00") into (date, "HH:MM").
pub fn split_dt(dt: &str) -> (String, String) {
    if let Some((date, time)) = dt.split_once('T') {
        (date.to_string(), time.chars().take(5).collect())
    } else {
        (dt.chars().take(10).collect(), "00:00".to_string())
    }
}

/// Stable dedupe key: (date, HH:MM, user_id, position_id, location_id).
pub fn spec_fingerprint(s: &PushSpec, home_location_id: i64) -> String {
    format!("{}|{}|{}|{}|{}", s.date, s.start, s.user_id, s.position_id, home_location_id)
}

/// Build the set of fingerprints already present at the home location.
/// Only planning + published shifts count (matches push_to_sling.py).
pub fn existing_fingerprints(events: &[CalendarEvent], home_location_id: i64) -> HashSet<String> {
    let mut out = HashSet::new();
    for ev in events {
        if ev.kind != "shift" { continue; }
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
```

(Also add `pub struct PushSpec { ... }` from the contract block above near the other DTOs so this compiles. Include `#[derive(Debug, Clone)]`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml fingerprint`
Expected: PASS (the `split_dt` and `existing_fingerprints` tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/sling.rs
git commit -m "feat(push): dedupe fingerprints + dtstart splitter"
```

---

## Task 3: `build_push_specs` — co-teach expansion + unassigned gate (sling.rs)

Turns proposal rows into POST specs. Skips dropped shifts. Hard-errors on an unassigned non-dropped shift. Expands a co-teach row (one row, `coteach_label = "Teacher A + Teacher E"`) into one spec per named teacher, resolving names via the roster map.

**Files:**
- Modify: `src-tauri/src/sling.rs`
- Test: same file

- [ ] **Step 1: Write the failing test**

```rust
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
    // 1 normal + 2 from co-teach + 0 dropped = 3
    assert_eq!(specs.len(), 3);
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml build_push_specs`
Expected: FAIL — `cannot find function build_push_specs`.

- [ ] **Step 3: Implement** (add to `sling.rs`)

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml build_push_specs`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/sling.rs
git commit -m "feat(push): build_push_specs with co-teach expansion + unassigned gate"
```

---

## Task 4: `build_shift_body` — POST body shape (sling.rs)

Locks the exact create-shift body. `users` is an **array**; `status` is the literal `"planning"`; `dtstart`/`dtend` are naive local (no offset).

**Files:**
- Modify: `src-tauri/src/sling.rs`
- Test: same file

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml build_shift_body`
Expected: FAIL — `cannot find function build_shift_body`.

- [ ] **Step 3: Implement** (add to `sling.rs`)

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml build_shift_body`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/sling.rs
git commit -m "feat(push): build_shift_body locking the create-shift contract"
```

---

## Task 5: `http_post`, `push_shift`, `fetch_calendar` (sling.rs)

The HTTP layer: a POST helper mirroring `http_get_with_query`, the single-shift create with 429 backoff, and a month calendar fetch for dedupe. These do real network I/O so they are exercised by the manual e2e, not unit tests.

**Files:**
- Modify: `src-tauri/src/sling.rs`

- [ ] **Step 1: Add `http_post`** (next to `http_get_with_query`)

```rust
/// POST JSON with browser-like headers + percent-encoded query params.
/// Returns parsed JSON on 2xx; maps known statuses to sentinel errors that
/// the command layer recognizes (sling-401, sling-429, sling-1010).
fn http_post(token: &str, url: &str, query: &[(&str, &str)], body: &serde_json::Value) -> Result<serde_json::Value> {
    let mut req = ureq::post(url)
        .set("Authorization", token)
        .set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .set("Origin", "https://app.getsling.com")
        .set("Referer", "https://app.getsling.com/")
        .set("Sec-Fetch-Dest", "empty")
        .set("Sec-Fetch-Mode", "cors")
        .set("Sec-Fetch-Site", "same-site")
        .set("Accept", "application/json, text/plain, */*");
    for (k, v) in query { req = req.query(k, v); }
    match req.send_json(body.clone()) {
        Ok(r) => Ok(r.into_json::<serde_json::Value>().unwrap_or(serde_json::Value::Null)),
        Err(ureq::Error::Status(401, _)) => Err(anyhow!("sling-401")),
        Err(ureq::Error::Status(429, _)) => Err(anyhow!("sling-429")),
        Err(ureq::Error::Status(1010, _)) => Err(anyhow!("sling-1010")),
        Err(ureq::Error::Status(code, r)) => {
            let b = r.into_string().unwrap_or_default();
            Err(anyhow!("sling-{code}: {b}"))
        }
        Err(e) => Err(anyhow!("sling-network: {e}")),
    }
}
```

- [ ] **Step 2: Add `push_shift`** (with 429 backoff, max 3 retries — never more)

```rust
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
```

- [ ] **Step 3: Add `fetch_calendar`** (month window for dedupe; reuses `month_range`'s `-05:00` form)

```rust
/// Fetch the target month's calendar events (for push dedupe). Mirrors the
/// pull's calendar GET: -05:00 offset, percent-encoded dates, nonce.
pub fn fetch_calendar(token: &str, cfg: &StudioConfig, month: &str) -> Result<Vec<CalendarEvent>> {
    let (start, end) = month_range(month)?;
    let url = format!("{BASE_URL}/{}/calendar/{}/users/{}", cfg.org_id, cfg.org_id, cfg.acting_user_id);
    let dates = format!("{start}/{end}");
    let nonce = chrono::Utc::now().timestamp_millis().to_string();
    let doc = http_get_with_query(token, &url, &[
        ("dates", &dates), ("user-fields", "id"), ("nonce", &nonce),
    ])?;
    let arr = doc.as_array().ok_or_else(|| anyhow!("calendar not array"))?;
    Ok(arr.iter().filter_map(|e| serde_json::from_value(e.clone()).ok()).collect())
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Finished with no errors (pre-existing `review.rs` dead-code warning is fine).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/sling.rs
git commit -m "feat(push): http_post, push_shift (429 backoff), fetch_calendar"
```

---

## Task 6: `build_specs_for_proposal` + `push_proposal_dry_run` command (commands.rs)

Loads proposal rows + roster + studio config from DuckDB, builds specs (running the gate), fetches Sling's calendar, dedupes, and returns the preview. Zero writes.

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Add response structs + the shared builder** (place after the Sling section, near `pull_month_from_sling`)

```rust
#[derive(serde::Serialize)]
pub struct PushPreviewItem { pub date: String, pub start: String, pub end: String, pub class_name: String, pub teacher_name: String }
#[derive(serde::Serialize)]
pub struct PushPreview { pub total: i64, pub skipped_count: i64, pub to_create: Vec<PushPreviewItem> }
#[derive(serde::Serialize, Clone)]
pub struct PushSummary { pub push_id: i64, pub created: i64, pub failed: i64, pub skipped: i64 }
#[derive(serde::Serialize, Clone)]
pub struct PushProgress { pub total: i64, pub done: i64, pub created: i64, pub failed: i64, pub skipped: i64, pub last_label: String, pub last_outcome: String }

/// Load proposal rows + roster map + studio config, then build the gated
/// push specs and the target month string. Shared by dry-run and execute.
fn build_specs_for_proposal(
    conn: &duckdb::Connection,
    proposal_id: i64,
) -> Result<(Vec<crate::sling::PushSpec>, crate::sling::StudioConfig, String), String> {
    let studio_cfg = load_studio_config(conn)?;
    let target_month: String = conn
        .query_row("SELECT target_month FROM proposals WHERE id = ?", duckdb::params![proposal_id], |r| r.get(0))
        .map_err(|e| format!("proposal {proposal_id} not found: {e}"))?;

    let name_to_id: std::collections::HashMap<String, i64> = {
        let mut stmt = conn.prepare("SELECT display_name, sling_user_id FROM teachers").map_err(err)?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i32>(1)? as i64)))
            .map_err(err)?.collect::<Result<_, _>>().map_err(err)?
    };

    let inputs: Vec<crate::sling::ProposalShiftInput> = {
        let mut stmt = conn.prepare(
            "SELECT ps.id, CAST(ps.shift_date AS VARCHAR), ps.start_time, ps.end_time,
                    ps.sling_position_id, ps.sling_user_id, t.display_name, pos.class_name,
                    ps.is_coteach, ps.coteach_label, ps.is_dropped
             FROM proposal_shifts ps
             JOIN positions pos ON pos.sling_position_id = ps.sling_position_id
             LEFT JOIN teachers t ON t.sling_user_id = ps.sling_user_id
             WHERE ps.proposal_id = ?
             ORDER BY ps.shift_date, ps.start_time"
        ).map_err(err)?;
        stmt.query_map(duckdb::params![proposal_id], |r| {
            let uid: Option<i32> = r.get(5)?;
            Ok(crate::sling::ProposalShiftInput {
                proposal_shift_id: r.get(0)?,
                date: r.get(1)?, start: r.get(2)?, end: r.get(3)?,
                position_id: r.get::<_, i32>(4)? as i64,
                user_id: uid.map(|u| u as i64),
                teacher_name: r.get(6)?,
                class_name: r.get(7)?,
                is_coteach: r.get(8)?,
                coteach_label: r.get(9)?,
                is_dropped: r.get(10)?,
            })
        }).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
    };

    let specs = crate::sling::build_push_specs(&inputs, &name_to_id)?;
    Ok((specs, studio_cfg, target_month))
}
```

- [ ] **Step 2: Add the dry-run command**

```rust
#[tauri::command]
pub fn push_proposal_dry_run(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
) -> Result<PushPreview, String> {
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — paste one in Settings".to_string())?
    };
    let (specs, cfg, month) = {
        let conn = db.0.lock().map_err(err)?;
        build_specs_for_proposal(&conn, proposal_id)?
    };
    let events = crate::sling::fetch_calendar(&token_str, &cfg, &month).map_err(err)?;
    let existing = crate::sling::existing_fingerprints(&events, cfg.home_location_id);

    let total = specs.len() as i64;
    let mut to_create = Vec::new();
    let mut skipped_count = 0i64;
    for s in &specs {
        if existing.contains(&crate::sling::spec_fingerprint(s, cfg.home_location_id)) {
            skipped_count += 1;
        } else {
            to_create.push(PushPreviewItem {
                date: s.date.clone(), start: s.start.clone(), end: s.end.clone(),
                class_name: s.class_name.clone(), teacher_name: s.teacher_name.clone(),
            });
        }
    }
    Ok(PushPreview { total, skipped_count, to_create })
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Finished, no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat(push): push_proposal_dry_run command + shared spec builder"
```

---

## Task 7: `push_proposal_execute` command (commands.rs)

Re-dedupes, inserts a `pushes` row, batch-POSTs with throttle, writes a `push_results` row per attempt, emits `push-progress`, and returns the summary. Aborts on 401.

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Add batch constants + the command**

```rust
const PUSH_BATCH_SIZE: usize = 10;
const PUSH_INTRA_DELAY_SECS: u64 = 1;
const PUSH_INTER_DELAY_SECS: u64 = 10;

#[tauri::command]
pub fn push_proposal_execute(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
) -> Result<PushSummary, String> {
    use tauri::Emitter;

    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — paste one in Settings".to_string())?
    };
    let (specs, cfg, month) = {
        let conn = db.0.lock().map_err(err)?;
        build_specs_for_proposal(&conn, proposal_id)?
    };
    let (viewdates, cachedates) = crate::sling::view_cache_dates(&month).map_err(err)?;

    // Re-dedupe at execute time (idempotent re-push: only POST what's missing).
    let events = crate::sling::fetch_calendar(&token_str, &cfg, &month).map_err(err)?;
    let existing = crate::sling::existing_fingerprints(&events, cfg.home_location_id);
    let to_create: Vec<&crate::sling::PushSpec> = specs.iter()
        .filter(|s| !existing.contains(&crate::sling::spec_fingerprint(s, cfg.home_location_id)))
        .collect();
    let skipped = (specs.len() - to_create.len()) as i64;
    let total = to_create.len() as i64;

    // Open the audit row.
    let push_id: i64 = {
        let conn = db.0.lock().map_err(err)?;
        conn.query_row(
            "INSERT INTO pushes (proposal_id, shifts_attempted, shifts_skipped) VALUES (?, ?, ?) RETURNING id",
            duckdb::params![proposal_id, total, skipped],
            |r| r.get(0),
        ).map_err(err)?
    };

    let mut created = 0i64;
    let mut failed = 0i64;
    let mut aborted_401 = false;

    'outer: for (idx, chunk) in to_create.chunks(PUSH_BATCH_SIZE).enumerate() {
        for (j, s) in chunk.iter().enumerate() {
            let label = format!("{} {} {} → {}", s.date, s.start, s.class_name, s.teacher_name);
            let (outcome, sling_id, errmsg): (&str, Option<String>, Option<String>) =
                match crate::sling::push_shift(&token_str, &cfg, s, &viewdates, &cachedates) {
                    Ok(id) => { created += 1; ("created", Some(id.to_string()), None) }
                    Err(e) if e.to_string() == "sling-401" => { aborted_401 = true; ("failed", None, Some("token expired".into())) }
                    Err(e) => { failed += 1; ("failed", None, Some(e.to_string())) }
                };
            {
                let conn = db.0.lock().map_err(err)?;
                conn.execute(
                    "INSERT INTO push_results (push_id, proposal_shift_id, outcome, sling_shift_id, error_message)
                     VALUES (?, ?, ?, ?, ?)",
                    duckdb::params![push_id, s.proposal_shift_id, outcome, sling_id, errmsg],
                ).map_err(err)?;
            }
            let done = created + failed;
            let _ = app.emit("push-progress", PushProgress {
                total, done, created, failed, skipped,
                last_label: label, last_outcome: outcome.to_string(),
            });
            if aborted_401 { break 'outer; }
            if j < chunk.len() - 1 {
                std::thread::sleep(std::time::Duration::from_secs(PUSH_INTRA_DELAY_SECS));
            }
        }
        if idx < to_create.len().div_ceil(PUSH_BATCH_SIZE) - 1 {
            std::thread::sleep(std::time::Duration::from_secs(PUSH_INTER_DELAY_SECS));
        }
    }

    // Close the audit row.
    {
        let conn = db.0.lock().map_err(err)?;
        conn.execute(
            "UPDATE pushes SET finished_at = now(), shifts_succeeded = ?, shifts_failed = ? WHERE id = ?",
            duckdb::params![created, failed, push_id],
        ).map_err(err)?;
    }

    if aborted_401 {
        return Err(format!("sling-401: token expired after creating {created} shift(s)"));
    }
    Ok(PushSummary { push_id, created, failed, skipped })
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Finished, no errors. (If `div_ceil` is unstable on the toolchain, replace `to_create.len().div_ceil(PUSH_BATCH_SIZE)` with `((to_create.len() + PUSH_BATCH_SIZE - 1) / PUSH_BATCH_SIZE)`.)

- [ ] **Step 3: Run the full Rust test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS (all existing tests + the new sling.rs tests from Tasks 1–4).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat(push): push_proposal_execute (batched, audited, idempotent, 401-abort)"
```

---

## Task 8: Register commands (lib.rs)

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add to the `invoke_handler` list** (after `commands::add_teacher_from_pull,`)

```rust
            commands::add_teacher_from_pull,
            commands::push_proposal_dry_run,
            commands::push_proposal_execute,
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Finished, no errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(push): register push commands"
```

---

## Task 9: Frontend types + API (types.ts, api.ts)

**Files:**
- Modify: `src/types.ts`
- Modify: `src/lib/api.ts`

- [ ] **Step 1: Add types** to `src/types.ts`

```ts
export interface PushPreviewItem { date: string; start: string; end: string; class_name: string; teacher_name: string; }
export interface PushPreview { total: number; skipped_count: number; to_create: PushPreviewItem[]; }
export interface PushSummary { push_id: number; created: number; failed: number; skipped: number; }
export interface PushProgress { total: number; done: number; created: number; failed: number; skipped: number; last_label: string; last_outcome: string; }
```

- [ ] **Step 2: Add the imports + API methods** in `src/lib/api.ts` (extend the existing type import block, then add methods before the closing `};`)

```ts
  PushPreview,
  PushSummary,
```

```ts
  pushProposalDryRun: (proposalId: number) =>
    invoke<PushPreview>("push_proposal_dry_run", { proposalId }),
  pushProposalExecute: (proposalId: number) =>
    invoke<PushSummary>("push_proposal_execute", { proposalId }),
```

- [ ] **Step 3: Verify the frontend type-checks**

Run: `npm run build`
Expected: build succeeds (tsc + vite).

- [ ] **Step 4: Commit**

```bash
git add src/types.ts src/lib/api.ts
git commit -m "feat(push): frontend types + api wrappers"
```

---

## Task 10: `PushModal` component (PushModal.tsx)

Preview → Confirm → live progress → summary, in one modal. Listens to `push-progress`. Routes 401 to a callback.

**Files:**
- Create: `src/components/PushModal.tsx`

- [ ] **Step 1: Create the component**

```tsx
import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type { PushPreview, PushProgress, PushSummary } from "../types";

interface Props {
  proposalId: number;
  monthLabel: string;
  onClose: () => void;
  onTokenExpired: () => void;
}

type Phase = "loading" | "preview" | "pushing" | "done" | "error";

export function PushModal({ proposalId, monthLabel, onClose, onTokenExpired }: Props) {
  const [phase, setPhase] = useState<Phase>("loading");
  const [preview, setPreview] = useState<PushPreview | null>(null);
  const [progress, setProgress] = useState<PushProgress | null>(null);
  const [summary, setSummary] = useState<PushSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const unlisten = useRef<(() => void) | null>(null);

  // Dry-run on mount.
  useEffect(() => {
    let cancelled = false;
    api.pushProposalDryRun(proposalId)
      .then((p) => { if (!cancelled) { setPreview(p); setPhase("preview"); } })
      .catch((e) => {
        if (!cancelled) {
          if (String(e).includes("sling-401")) onTokenExpired();
          else { setError(String(e)); setPhase("error"); }
        }
      });
    return () => { cancelled = true; };
  }, [proposalId]);

  // Subscribe to progress before executing; clean up on unmount.
  useEffect(() => {
    listen<PushProgress>("push-progress", (e) => setProgress(e.payload)).then((u) => { unlisten.current = u; });
    return () => { unlisten.current?.(); };
  }, []);

  const onConfirm = async () => {
    setPhase("pushing");
    setError(null);
    try {
      const s = await api.pushProposalExecute(proposalId);
      setSummary(s);
      setPhase("done");
    } catch (e) {
      if (String(e).includes("sling-401")) onTokenExpired();
      else { setError(String(e)); setPhase("error"); }
    }
  };

  const pct = progress && progress.total > 0
    ? Math.round((progress.done / progress.total) * 100) : 0;

  return (
    <div className="modal-backdrop" onClick={phase === "pushing" ? undefined : onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>Push {monthLabel} to Sling</h3>

        {phase === "loading" && <p className="muted">Checking what’s already in Sling…</p>}

        {phase === "preview" && preview && (
          <>
            <p>
              <strong>{preview.to_create.length}</strong> shift(s) will be created as planning shifts.
              {preview.skipped_count > 0 && <> <span className="muted">{preview.skipped_count} already in Sling (skipped).</span></>}
            </p>
            {preview.to_create.length === 0 ? (
              <p className="ok">Everything is already in Sling — nothing to push.</p>
            ) : (
              <div className="bk-push-list">
                {preview.to_create.map((it, i) => (
                  <div className="bk-push-row" key={i}>
                    <span>{it.date} {it.start}–{it.end}</span>
                    <span>{it.class_name}</span>
                    <span>→ {it.teacher_name}</span>
                  </div>
                ))}
              </div>
            )}
            <div className="row" style={{ justifyContent: "space-between", marginTop: 12 }}>
              <button className="btn-ghost" onClick={onClose}>Cancel</button>
              <button className="btn-primary" onClick={onConfirm} disabled={preview.to_create.length === 0}>
                Confirm push ({preview.to_create.length})
              </button>
            </div>
          </>
        )}

        {phase === "pushing" && (
          <>
            <p className="muted">Pushing in batches of 10 with pauses (Sling rate-limits). Keep this window open.</p>
            <div className="bk-progress"><div className="bk-progress-fill" style={{ width: `${pct}%` }} /></div>
            {progress && (
              <p className="muted">
                {progress.done}/{progress.total} — {progress.created} created
                {progress.failed > 0 && <>, {progress.failed} failed</>}
                {progress.last_label && <><br /><code>{progress.last_outcome}: {progress.last_label}</code></>}
              </p>
            )}
          </>
        )}

        {phase === "done" && summary && (
          <>
            <p className="ok"><strong>Done.</strong> {summary.created} created, {summary.failed} failed, {summary.skipped} already present.</p>
            {summary.failed > 0 && <p className="muted">Some shifts failed — click Push again to retry; the ones already created are skipped automatically.</p>}
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 12 }}>
              <button className="btn-primary" onClick={onClose}>Close</button>
            </div>
          </>
        )}

        {phase === "error" && (
          <>
            <div className="error">{error}</div>
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 12 }}>
              <button className="btn-ghost" onClick={onClose}>Close</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify it type-checks**

Run: `npm run build`
Expected: build succeeds.

- [ ] **Step 3: Commit**

```bash
git add src/components/PushModal.tsx
git commit -m "feat(push): PushModal (preview, progress, summary)"
```

---

## Task 11: Wire the toolbar button into the proposal detail (App.tsx)

**Files:**
- Modify: `src/App.tsx`

- [ ] **Step 1: Import the component** (with the other component imports near the top of `App.tsx`)

```tsx
import { PushModal } from "./components/PushModal";
```

- [ ] **Step 2: Add modal state** (next to the other `useState` calls in the proposals component, e.g. beside `slingExpiredModal`)

```tsx
  const [pushOpen, setPushOpen] = useState(false);
```

- [ ] **Step 3: Add the Push toolbar** inside the `{detail && (` block, immediately before `<div className="bk-tabs">`

```tsx
          <div className="row" style={{ justifyContent: "flex-end", marginBottom: 8 }}>
            <button
              className="btn-primary"
              onClick={() => setPushOpen(true)}
              disabled={isReadOnlyMonth(detail.summary.target_month, today)}
              title={isReadOnlyMonth(detail.summary.target_month, today) ? "Past month — read only" : "Push these shifts to Sling as planning shifts"}
            >
              Push to Sling
            </button>
          </div>
```

- [ ] **Step 4: Render the modal** immediately after the existing `{slingExpiredModal && ( ... )}` block

```tsx
      {pushOpen && detail && (
        <PushModal
          proposalId={detail.summary.id}
          monthLabel={detail.summary.target_month}
          onClose={() => { setPushOpen(false); onProposalChanged(); }}
          onTokenExpired={() => { setPushOpen(false); setSlingExpiredModal(true); }}
        />
      )}
```

- [ ] **Step 5: Verify it type-checks**

Run: `npm run build`
Expected: build succeeds.

- [ ] **Step 6: Commit**

```bash
git add src/App.tsx
git commit -m "feat(push): Push to Sling button + modal wiring in proposal view"
```

---

## Task 12: Styles + docs

**Files:**
- Modify: `src/styles.css`
- Modify: `docs/architecture.md`

- [ ] **Step 1: Add styles** to the end of `src/styles.css`

```css
/* Push modal */
.bk-push-list { max-height: 280px; overflow-y: auto; border: 1px solid var(--color-border); border-radius: var(--radius); padding: 4px 0; margin: 8px 0; }
.bk-push-row { display: grid; grid-template-columns: 1.4fr 1fr 1fr; gap: 8px; padding: 4px 10px; font-size: 13px; }
.bk-push-row:nth-child(odd) { background: var(--color-bg-subtle, #f6f6f6); }
.bk-progress { height: 10px; background: var(--color-bg-subtle, #eee); border-radius: 999px; overflow: hidden; margin: 10px 0; }
.bk-progress-fill { height: 100%; background: var(--color-accent, #6b4eff); transition: width 0.3s ease; }
```

- [ ] **Step 2: Update the push step description** in `docs/architecture.md` (find the line describing step 5 "Push to Sling" that mentions calling `push_to_sling.py` with a CSV) and replace it with:

```markdown
5. **Push to Sling.** User clicks "Push to Sling" on a proposal. The app builds the shift list from `proposal_shifts` in DuckDB, dedupes against shifts already in Sling, and POSTs the missing ones in-process (Rust, `sling.rs::push_shift`) as `status: "planning"`, batched + rate-limit-aware. Audit goes to the `pushes` and `push_results` tables. (The legacy `scripts/push_to_sling.py` is retained for reference only and is no longer invoked.)
```

- [ ] **Step 3: Verify build still passes**

Run: `npm run build && cargo test --manifest-path src-tauri/Cargo.toml`
Expected: both succeed.

- [ ] **Step 4: Commit**

```bash
git add src/styles.css docs/architecture.md
git commit -m "feat(push): modal styles + architecture doc update"
```

---

## Manual end-to-end validation (after all tasks)

This is the validation round to run against a real Sling session before relying on the feature:

1. Settings → confirm Studio configuration (org / acting-user / home-location) is set and a fresh Sling token is pasted.
2. Pick an upcoming month → Pull from Sling → Generate proposal → resolve any unassigned shifts in the calendar.
3. Click **Push to Sling**. Confirm the preview counts look right (created vs already-present).
4. Click **Confirm push**. Watch the progress bar; let it finish.
5. In Sling's web UI, confirm the new shifts appear as **planning** status at the home location, with correct teachers/times (spot-check a co-teach slot for two records).
6. Click **Push to Sling** again → the preview should now show ~all skipped (idempotent dedupe).
7. (Optional) Let a token expire mid-run and confirm the expired-token modal appears and a re-push resumes cleanly.
```
