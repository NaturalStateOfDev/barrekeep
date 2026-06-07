# Studio Auto-Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **GIT SAFETY (a prior run lost work to a detached HEAD):** subagents must NEVER run `git checkout`/`switch`/`reset`/`restore`/`rebase`/`stash`. Only `git add` + `git commit` on the current branch. Inspect history with `git diff A..B` / `git show sha:path`. Before committing, verify `git rev-parse --abbrev-ref HEAD` is `feat/studio-autodiscover`.

**Goal:** After a Sling login, auto-discover the studio's org / acting-user / home-location IDs and prefill them into the Settings studio-config card for review-and-save.

**Architecture:** A pure-Rust discovery step in `sling.rs` (`account/session` + `users/concise` + `groups`) reached by a `discover_studio_config` Tauri command, run automatically when `sling-token-saved` fires (and via a manual "Detect from Sling" button). The login interceptor additionally parses `org_id` from the request URL into a new `SlingOrgHint` state as a guaranteed org fallback. Discovery only prefills the form — `set_studio_config` stays the sole writer.

**Tech Stack:** Rust (ureq, serde_json, Tauri 2 state), React + TypeScript.

**Spec:** `docs/superpowers/specs/2026-06-06-studio-autodiscover-design.md`

---

## File structure

- **`src-tauri/src/sling.rs`** (modify) — `DiscoveredStudio`/`DiscoveredLocation` structs, pure `parse_session` + `select_locations` helpers (unit-tested), and the `discover_studio` HTTP orchestration. Reuses `http_get`, `SlingUser`, `location_name_by_id`.
- **`src-tauri/src/commands.rs`** (modify) — `SlingOrgHint` state struct + `discover_studio_config` command.
- **`src-tauri/src/sling_login.rs`** (modify) — store the `o` (org) sentinel param into `SlingOrgHint` in the existing `on_navigation` handler (cheap mutex write; no lifecycle change).
- **`src-tauri/src/sling_login_capture.js`** (modify) — parse `org_id` from the intercepted `/v1/{org}/…` request URL and append `&o=<org>` to the sentinel.
- **`src-tauri/src/lib.rs`** (modify) — manage `SlingOrgHint` state; register `discover_studio_config`.
- **`src/types.ts` / `src/lib/api.ts`** (modify) — `DiscoveredStudio`/`DiscoveredLocation` + `discoverStudioConfig()`.
- **`src/App.tsx`** (modify) — `StudioConfigCard`: auto-detect on `sling-token-saved`, a "Detect from Sling" button, a location dropdown when detections exist.

### Type/signature contract (keep names exact across tasks)

```rust
// sling.rs
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredLocation { pub id: i64, pub name: String }
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredStudio {
    pub org_id: i64,
    pub acting_user_id: i64,
    pub acting_user_name: String,
    pub locations: Vec<DiscoveredLocation>,
}
pub fn parse_session(v: &serde_json::Value) -> anyhow::Result<(i64, String, Option<i64>)>; // (user_id, name, org_from_session)
pub fn select_locations(group_ids: &[i64], loc_names: &std::collections::HashMap<i64, String>) -> Vec<DiscoveredLocation>;
pub fn discover_studio(token: &str, org_hint: Option<i64>) -> anyhow::Result<DiscoveredStudio>;
```

```ts
// types.ts
export interface DiscoveredLocation { id: number; name: string; }
export interface DiscoveredStudio { org_id: number; acting_user_id: number; acting_user_name: string; locations: DiscoveredLocation[]; }
```

---

## Task 1: Discovery DTOs + `parse_session` (sling.rs)

**Files:**
- Modify: `src-tauri/src/sling.rs`
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Add the structs** near the other DTOs (e.g. after `PushSpec`)

```rust
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredLocation { pub id: i64, pub name: String }

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredStudio {
    pub org_id: i64,
    pub acting_user_id: i64,
    pub acting_user_name: String,
    pub locations: Vec<DiscoveredLocation>,
}
```

(`Serialize` is already imported at the top: `use serde::{Deserialize, Serialize};`.)

- [ ] **Step 2: Write the failing tests** inside the existing `#[cfg(test)] mod tests` block

```rust
#[test]
fn parse_session_reads_user_and_optional_org() {
    // user id may come back as a number or a string; org may be absent.
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
fn parse_session_errors_without_user() {
    let v = serde_json::json!({ "nope": true });
    assert!(parse_session(&v).is_err());
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib parse_session`
Expected: FAIL — `cannot find function parse_session`.

- [ ] **Step 4: Implement** (add to `sling.rs`)

```rust
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
    // Tolerant org-id search across the likely shapes.
    let org = ["org", "organization"].iter()
        .find_map(|k| v.get(*k).and_then(|o| o.get("id")).and_then(json_i64))
        .or_else(|| v.get("orgId").and_then(json_i64))
        .or_else(|| user.get("orgId").and_then(json_i64))
        .or_else(|| user.get("org").and_then(|o| o.get("id")).and_then(json_i64));
    Ok((uid, name, org))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib parse_session`
Expected: PASS (2 tests). Also run `cargo test --manifest-path src-tauri/Cargo.toml --lib` → all prior tests still pass.

- [ ] **Step 6: Commit** (confirm branch first: `git rev-parse --abbrev-ref HEAD` → `feat/studio-autodiscover`)

```bash
git add src-tauri/src/sling.rs
git commit -m "feat(discover): DiscoveredStudio DTOs + parse_session"
```

---

## Task 2: `select_locations` + `discover_studio` (sling.rs)

**Files:**
- Modify: `src-tauri/src/sling.rs`
- Test: same file

- [ ] **Step 1: Write the failing test** (inside `mod tests`)

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib select_locations`
Expected: FAIL — `cannot find function select_locations`.

- [ ] **Step 3: Implement `select_locations`** (add to `sling.rs`)

```rust
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
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib select_locations`
Expected: PASS.

- [ ] **Step 5: Implement `discover_studio`** (HTTP orchestration; no unit test — exercised manually). Add to `sling.rs`:

```rust
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

    Ok(DiscoveredStudio {
        org_id, acting_user_id, acting_user_name,
        locations: select_locations(&group_ids, &loc_names),
    })
}
```

- [ ] **Step 6: Verify it compiles + all tests pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib`
Expected: PASS (existing + the 3 new discovery tests).

- [ ] **Step 7: Commit** (confirm branch first)

```bash
git add src-tauri/src/sling.rs
git commit -m "feat(discover): select_locations + discover_studio orchestration"
```

---

## Task 3: `SlingOrgHint` state + capture org from login URL

**Files:**
- Modify: `src-tauri/src/commands.rs` (state struct)
- Modify: `src-tauri/src/sling_login_capture.js`
- Modify: `src-tauri/src/sling_login.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the state struct** in `commands.rs` next to `SlingToken` (search for `pub struct SlingToken(pub Mutex<Option<String>>);`) and add below it:

```rust
/// Org id opportunistically parsed from the Sling login request URL, used as a
/// fallback when account/session doesn't expose it. See sling_login.rs.
pub struct SlingOrgHint(pub Mutex<Option<i64>>);
```

- [ ] **Step 2: Capture org in the JS interceptor.** In `src-tauri/src/sling_login_capture.js`, find the `tryCapture` function's success block (where it builds `const target = CAPTURE_URL + "?t=" + ...` and calls `window.location.replace(target)`). Replace those two lines with:

```js
      let target = CAPTURE_URL + "?t=" + encodeURIComponent(String(authHeader));
      // Opportunistically grab the org id from an org-scoped /v1/{org}/… URL
      // so the app can prefill studio config. Absent on non-org endpoints.
      const orgMatch = u.pathname.match(/^\/v1\/(\d+)(?:\/|$)/);
      if (orgMatch) target += "&o=" + orgMatch[1];
      window.location.replace(target);
```

- [ ] **Step 3: Store the hint in the nav handler.** In `src-tauri/src/sling_login.rs`, find the block in `on_navigation` that writes the in-memory token (`if let Some(state) = app_for_nav.try_state::<SlingToken>() { ... }`). Immediately after that block, add:

```rust
        // Stash the org hint (cheap mutex write — safe inside on_navigation).
        if let Some(org) = url.query_pairs()
            .find(|(k, _)| k == "o")
            .and_then(|(_, v)| v.parse::<i64>().ok())
        {
            if let Some(hint) = app_for_nav.try_state::<crate::commands::SlingOrgHint>() {
                if let Ok(mut g) = hint.0.lock() { *g = Some(org); }
            }
        }
```

(Do NOT change the deferred-persistence thread or the `run_on_main_thread` close — only add the above.)

- [ ] **Step 4: Manage the state** in `src-tauri/src/lib.rs`. Find `app.manage(SlingToken(Mutex::new(initial_token)));` and add immediately after:

```rust
            app.manage(commands::SlingOrgHint(Mutex::new(None)));
```

(If `commands::SlingOrgHint` isn't resolvable, check the existing `use commands::{AnthropicKey, SlingToken};` line and add `SlingOrgHint` to it; either the path-qualified form or the import works.)

- [ ] **Step 5: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Finished, no errors.

- [ ] **Step 6: Commit** (confirm branch first)

```bash
git add src-tauri/src/commands.rs src-tauri/src/sling_login_capture.js src-tauri/src/sling_login.rs src-tauri/src/lib.rs
git commit -m "feat(discover): capture org_id from login URL into SlingOrgHint"
```

---

## Task 4: `discover_studio_config` command + registration

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the command** in `commands.rs` (place near `open_sling_login_window`)

```rust
#[tauri::command]
pub fn discover_studio_config(
    token: State<'_, SlingToken>,
    org_hint: State<'_, SlingOrgHint>,
) -> Result<crate::sling::DiscoveredStudio, String> {
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — log in to Sling first".to_string())?
    };
    let hint = { *org_hint.0.lock().map_err(err)? };
    crate::sling::discover_studio(&token_str, hint).map_err(err)
}
```

- [ ] **Step 2: Register it** in `src-tauri/src/lib.rs` invoke handler — find `commands::open_sling_login_window,` and add after it:

```rust
            commands::discover_studio_config,
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Finished, no errors (no more dead-code warning on `discover_studio`).

- [ ] **Step 4: Commit** (confirm branch first)

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(discover): discover_studio_config command"
```

---

## Task 5: Frontend types + API

**Files:**
- Modify: `src/types.ts`
- Modify: `src/lib/api.ts`

- [ ] **Step 1: Add types** to `src/types.ts` (near the other interfaces)

```ts
export interface DiscoveredLocation { id: number; name: string; }
export interface DiscoveredStudio {
  org_id: number;
  acting_user_id: number;
  acting_user_name: string;
  locations: DiscoveredLocation[];
}
```

- [ ] **Step 2: Add the import + method** in `src/lib/api.ts`. Add `DiscoveredStudio` to the `import type { … } from "../types";` block, then add this method (e.g. after `getStudioConfig`/`setStudioConfig`):

```ts
  discoverStudioConfig: () => invoke<DiscoveredStudio>("discover_studio_config"),
```

- [ ] **Step 3: Verify the frontend type-checks**

Run: `npm run build`
Expected: build succeeds.

- [ ] **Step 4: Commit** (confirm branch first)

```bash
git add src/types.ts src/lib/api.ts
git commit -m "feat(discover): frontend types + discoverStudioConfig api"
```

---

## Task 6: Wire detection into `StudioConfigCard`

**Files:**
- Modify: `src/App.tsx` (the `StudioConfigCard` function, currently starting at `function StudioConfigCard() {`)

Detection auto-runs on `sling-token-saved`, plus a manual "Detect from Sling" button. When locations are detected, the home-location field becomes a dropdown of those options. Org and acting-user fields are prefilled. Nothing is saved without clicking Save.

- [ ] **Step 1: Add imports/state.** At the top of `StudioConfigCard`, after the existing `const [error, setError] = useState<string | null>(null);`, add:

```tsx
  const [locations, setLocations] = useState<DiscoveredLocation[]>([]);
  const [detecting, setDetecting] = useState(false);
  const [detectMsg, setDetectMsg] = useState<string | null>(null);
```

Ensure `DiscoveredLocation` is imported. At the top of `App.tsx` find the `import type { … } from "./types";` block and add `DiscoveredLocation`. Also confirm `listen` is imported from `@tauri-apps/api/event` (it is used elsewhere in App.tsx); if not, add `import { listen } from "@tauri-apps/api/event";`.

- [ ] **Step 2: Add the detect function** inside `StudioConfigCard`, after the `refresh` definition:

```tsx
  const detect = async () => {
    setDetecting(true);
    setDetectMsg(null);
    setError(null);
    try {
      const d = await api.discoverStudioConfig();
      setOrgId(String(d.org_id));
      setActingUserId(String(d.acting_user_id));
      setLocations(d.locations);
      if (d.locations.length === 1) setHomeLocationId(String(d.locations[0].id));
      setDetectMsg(
        `Detected ${d.acting_user_name || "your"} studio (org ${d.org_id}). ` +
        `Pick the home location and click Save.`,
      );
    } catch (e) {
      const msg = String(e);
      if (msg.includes("sling-401")) {
        setDetectMsg("Sling token expired — use “Log in to Sling” above to refresh, then Detect again.");
      } else if (msg.includes("no Sling token")) {
        setDetectMsg("Log in to Sling first (card above), then Detect.");
      } else {
        setDetectMsg("Couldn’t auto-detect everything — enter the IDs manually below.");
      }
    } finally {
      setDetecting(false);
    }
  };
```

- [ ] **Step 3: Auto-run on login.** Add this effect after the existing `useEffect(() => { refresh(); }, []);`:

```tsx
  useEffect(() => {
    const p = listen<void>("sling-token-saved", () => { detect(); });
    return () => { p.then((un) => un()); };
  }, []);
```

- [ ] **Step 4: Replace the home-location input with a conditional dropdown.** Find the existing home-location `<label className="field">` block:

```tsx
      <label className="field" style={{ marginTop: 8 }}>
        <span>Home location id</span>
        <input type="number" min={0} value={homeLocationId} onChange={(e) => setHomeLocationId(e.target.value)} placeholder="0" style={fieldStyle} />
      </label>
```

Replace it with:

```tsx
      <label className="field" style={{ marginTop: 8 }}>
        <span>Home location{locations.length > 0 ? "" : " id"}</span>
        {locations.length > 0 ? (
          <select value={homeLocationId} onChange={(e) => setHomeLocationId(e.target.value)} style={fieldStyle}>
            <option value="">— pick your studio —</option>
            {locations.map((l) => (
              <option key={l.id} value={String(l.id)}>{l.name} ({l.id})</option>
            ))}
          </select>
        ) : (
          <input type="number" min={0} value={homeLocationId} onChange={(e) => setHomeLocationId(e.target.value)} placeholder="0" style={fieldStyle} />
        )}
      </label>
```

- [ ] **Step 5: Add the Detect button + message.** Find the actions row:

```tsx
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={onSave}>Save</button>
      </div>
```

Replace it with:

```tsx
      <div className="row" style={{ marginTop: 12, gap: 8 }}>
        <button className="btn-primary" onClick={onSave}>Save</button>
        <button className="btn-ghost" onClick={detect} disabled={detecting}>
          {detecting ? "Detecting…" : "Detect from Sling"}
        </button>
      </div>
      {detectMsg && <div className="muted" style={{ marginTop: 8 }}>{detectMsg}</div>}
```

- [ ] **Step 6: Verify build**

Run: `npm run build`
Expected: build succeeds.

- [ ] **Step 7: Commit** (confirm branch first)

```bash
git add src/App.tsx
git commit -m "feat(discover): auto-detect studio config in Settings + location dropdown"
```

---

## Manual end-to-end validation (after all tasks)

1. Settings → "Log in to Sling" → complete login. On `sling-token-saved`, the studio-config card should auto-populate org + acting-user and show a location dropdown of your detected location(s).
2. Confirm the dropdown lists the right studio; pick it; click Save; status flips to "configured".
3. Run a Pull for an upcoming month — it should target the saved studio.
4. Click "Detect from Sling" again to confirm the manual path re-detects.
5. (Edge) With an expired/cleared token, click Detect → expect the "token expired / log in first" message, not a crash.
