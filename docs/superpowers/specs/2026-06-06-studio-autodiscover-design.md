# Auto-discover studio config on login — design

**Date:** 2026-06-06
**Status:** approved (design); implementation plan to follow
**Touches:** Sling integration (read `docs/sling-api.md` + the `sling-integration` skill before coding), the browser-login flow, and the Settings studio-config card.

## Problem

The Sling org id, acting-user id, and home-location id live in `studio_config`
(migration 0007) and must be entered by hand in Settings → Studio
configuration. A pull/push errors until they're set. The values are
discoverable from Sling once authenticated, so the app should detect and
prefill them as part of logging in to get the token — turning a fiddly
DevTools-spelunking step into a review-and-save.

## Decision

When a Sling login completes, the app auto-runs a **discovery** step that reads
the studio identifiers from Sling and **prefills** them into the Settings
studio-config card for the user to review and **Save**. Discovery never writes
`studio_config` on its own (prefill-and-confirm, not auto-save) and never
silently overwrites an existing saved config.

Approach: **pure post-login Rust discovery** (no changes to the
deadlock-sensitive login nav path beyond reading a string), with the org id
captured opportunistically from the login request URL as a guaranteed fallback.

### Decisions locked during brainstorming
- **Prefill + confirm** (not silent auto-save): discovery populates the form; the
  user clicks Save. (Q1)
- **Always show a location dropdown** of the detected locations, even when there
  is exactly one, so the home location is always an explicit choice. (Q2)
- Discovery runs **automatically after login** and is also available via a manual
  **"Detect from Sling"** button in the studio-config card.

### Approaches considered
1. **Pure post-login Rust discovery (chosen)** — login unchanged; a Rust command
   calls Sling after `sling-token-saved`. Clean separation, unit-testable.
   Leans on `account/session` for the org id, with the login-URL `org_hint` as a
   guaranteed fallback.
2. Capture org/user from the intercepted login request URL — bulletproof org id
   but touches the just-hardened login path, and the user id isn't reliably in
   the first captured request.
3. Manual "Detect" button only — simplest, but not tied to login as requested.

## Data flow

```
Login webview captures the bearer token (unchanged deferred-close behavior) AND
  the capture JS parses org_id from the authenticated /v1/{org}/… request URL it
  inspects, appending &o=<org> to the sentinel. on_navigation stores it into a
  new SlingOrgHint(Mutex<Option<i64>>) state. (String parsing only — no window
  lifecycle work in the handler.)
        │  emits sling-token-saved
        ▼
Studio-config card (Settings) hears sling-token-saved → calls discover_studio_config()
        ▼
Rust discover_studio(token, org_hint):                       [sling.rs]
   GET /v1/account/session  → acting_user_id, acting_user_name, org_id (else org_hint)
   GET /v1/users/concise    → the acting user's groupIds
   GET /v1/groups           → location-type groups (id, name)
   intersect groupIds ∩ location groups → the lead's location memberships
   → DiscoveredStudio { org_id, acting_user_id, acting_user_name, locations: [{id,name}] }
        ▼
Card PREFILLS org + acting-user fields and renders a location dropdown (always).
        ▼
User reviews → Save → existing set_studio_config command (the only writer).
```

## Components

- **`src-tauri/src/sling.rs`**
  - `pub struct DiscoveredStudio { org_id: i64, acting_user_id: i64, acting_user_name: String, locations: Vec<DiscoveredLocation> }` and `pub struct DiscoveredLocation { id: i64, name: String }` (serde Serialize).
  - `pub fn discover_studio(token: &str, org_hint: Option<i64>) -> Result<DiscoveredStudio>`:
    1. Determine the org id and acting user. `account/session` may be org-scoped
       (`scripts/sling_extract.py` calls `/{org}/account/session`), so:
       - If `org_hint` is `Some`, use it as the org id and call
         `GET /v1/{org}/account/session` for the acting user id + name.
       - If `org_hint` is `None`, call the bare `GET /v1/account/session` and read
         both the acting user (id, name) and the org id from the response.
       - If no org id can be determined either way, return an error
         ("couldn't determine Sling org — enter it manually").
       Org-id/user extraction from the session JSON is tolerant (see
       `parse_session` below); the exact paths are confirmed against a captured
       response during implementation, and `org_hint` guarantees a working result.
    2. `GET /v1/users/concise` → find the acting user by id → their `group_ids` (reuses the existing `SlingUser` DTO).
    3. `GET /v1/groups` → `location_name_by_id` (existing helper) for location-type groups.
    4. `locations` = the acting user's `group_ids` that are location groups, mapped to `{id, name}`, sorted by name. If the user has no location memberships, fall back to listing all org location groups (so the dropdown is never empty).
  - A small pure helper `parse_session(value: &serde_json::Value, org_hint: Option<i64>) -> Result<(i64 user_id, String name, Option<i64> org_id)>` so the JSON-shape parsing is unit-testable without HTTP. Org-id extraction is tolerant (checks the likely locations in the session JSON); the exact path is confirmed against a captured response during implementation, and `org_hint` guarantees a working result regardless.
- **`src-tauri/src/sling_login.rs` + `sling_login_capture.js`**
  - Capture JS: when it fires on the first authenticated request, parse the org id from the request URL (`/v1/{orgId}/…`, when org-scoped) and append `&o=<orgId>` to the sentinel URL (omit when the captured request isn't org-scoped).
  - `on_navigation`: read the optional `o` query param and store it into the new `SlingOrgHint` state. This is added alongside the existing token handling; the deferred persistence + `run_on_main_thread` close stay exactly as they are.
- **`src-tauri/src/commands.rs`**
  - `#[tauri::command] pub fn discover_studio_config(token: State<SlingToken>, org_hint: State<SlingOrgHint>) -> Result<DiscoveredStudio, String>`: clones the token (err if absent), reads the org hint, calls `sling::discover_studio`.
- **`src-tauri/src/lib.rs`**
  - Register `SlingOrgHint(Mutex<Option<i64>>)` in managed state; register `discover_studio_config` in the invoke handler.
- **`src/types.ts` / `src/lib/api.ts`**
  - `DiscoveredStudio` + `DiscoveredLocation` types; `discoverStudioConfig(): Promise<DiscoveredStudio>`.
- **Studio-config card (Settings, in `src/App.tsx`)**
  - On mount-time `sling-token-saved` event, auto-run `discoverStudioConfig()` (best-effort). On success, prefill the org and acting-user inputs and populate a location `<select>` from `locations` (no auto-Save).
  - A manual **"Detect from Sling"** button runs the same discovery on demand.
  - Save uses the existing `set_studio_config` path unchanged.

## Error handling (best-effort, never blocks, never silently overwrites)

- **401** during discovery → surface the existing `SlingTokenModal` (reason "expired"); discovery is abandoned, manual entry still works.
- **Partial detection** (session shape unexpected, no locations, etc.) → prefill whatever resolved, leave the rest blank/manual, and show a muted "couldn't auto-detect everything — fill in manually" note.
- Discovery **never** calls `set_studio_config`; the user always clicks Save.
- An existing saved config is prefilled-over in the *form* only; the stored row changes only on Save.

## Testing

- **Rust unit tests** (no HTTP): `parse_session` extracts user id/name + org id from a representative fixture; org-hint fallback path when the session has no org id; the location-intersection logic (user in one location, user in several → multi-entry list, user in none → falls back to all location groups).
- **Manual end-to-end:** fresh login → Settings shows org + acting-user prefilled and a location dropdown of the detected locations → pick → Save → a pull succeeds against the saved config. Also: re-run via the "Detect from Sling" button; and confirm an expired token mid-discovery pops the token modal.

## Out of scope

- Auto-saving `studio_config` without confirmation.
- Discovering anything beyond org / acting-user / home-location (e.g. roster — that already arrives via the pull).
- Multi-org accounts: the studio is single-org; if `account/session` ever returns multiple orgs, discovery uses the org from `org_hint`/the first org and the user can correct it before Save.
