# Sling-sourced Roster + Positions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.
>
> **GIT SAFETY (a prior run lost work to a detached HEAD):** subagents must NEVER run `git checkout`/`switch`/`reset`/`restore`/`rebase`/`stash`. Only `git add` + `git commit` on the current branch. Inspect with `git diff A..B` / `git show sha:path`. Before committing, verify `git rev-parse --abbrev-ref HEAD` is `feat/roster-from-sling`.
>
> Migration work follows the `schema-change` skill (forward-only, idempotent). The algorithm-verification task follows the `schedule-algorithm` skill.

**Goal:** Make Sling the source of truth for the roster and class list — a shared `sync_roster` routine imports active, home-location, class-qualified teachers + positions + qualifications from Sling (on every month pull and via a Teachers-page "Refresh from Sling" button); the placeholder seed is removed and the manual add-teacher/candidates flow retires.

**Architecture:** A `sync_roster(conn, users, groups, cfg)` routine in `commands.rs` (pure decision helpers in `sling.rs`) upserts positions then teachers then qualifications inside one transaction, deactivating (never deleting) anything no longer in Sling. `pull_month_from_sling` and a new `refresh_roster_from_sling` command both call it. The proposer already excludes inactive teachers and flags unfillable slots, so the algorithm needs verification only.

**Tech Stack:** Rust (duckdb, serde), Tauri 2, React + TypeScript, Python (propose.py — verification only).

**Spec:** `docs/superpowers/specs/2026-06-06-roster-from-sling-design.md`

---

## File structure

- **`src-tauri/migrations/0008_drop_demo_roster.sql`** (create) — purge placeholder demo teachers (unreferenced) + their quals; drop `sling_candidates`.
- **`src-tauri/src/seed.rs`** (modify) — stop seeding teachers/positions/qualifications.
- **`src-tauri/src/sling.rs`** (modify) — pure helper `is_schedulable_teacher` + tests.
- **`src-tauri/src/commands.rs`** (modify) — `RosterSyncSummary`, `sync_roster`, `refresh_roster_from_sling`; rewrite the teacher/position/qual block of `pull_month_from_sling` to call `sync_roster`; remove `add_teacher_from_pull`, `list_sling_candidates`, `NewUserSummary` from the pull; add `set_position_active`.
- **`src-tauri/src/lib.rs`** (modify) — register `refresh_roster_from_sling`, `set_position_active`; unregister `add_teacher_from_pull`, `list_sling_candidates`.
- **`src/types.ts` / `src/lib/api.ts`** (modify) — `RosterSyncSummary` type, `refreshRosterFromSling`, `setPositionActive`; remove `listSlingCandidates`/`addTeacherFromPull`; drop `new_users` from `PullResult`.
- **`src/App.tsx`** (modify) — `TeachersView` (Refresh button, hide inactive, empty state, remove AddTeacherCard/candidates), `ProposalsView` (remove new-user banner), `PositionsView` (schedulable toggle).

### Contract (keep names exact)

```rust
// commands.rs
#[derive(serde::Serialize, Clone)]
pub struct RosterSyncSummary { pub teachers_active: i64, pub teachers_deactivated: i64, pub positions_active: i64, pub positions_deactivated: i64, pub qualifications: i64 }
fn sync_roster(conn: &duckdb::Connection, users: &[crate::sling::SlingUser], groups: &[crate::sling::SlingGroup], cfg: &crate::sling::StudioConfig) -> Result<RosterSyncSummary, String>;
```
```ts
// types.ts
export interface RosterSyncSummary { teachers_active: number; teachers_deactivated: number; positions_active: number; positions_deactivated: number; qualifications: number; }
```

---

## Task 1: Migration 0008 — purge demo roster

**Files:** Create `src-tauri/migrations/0008_drop_demo_roster.sql`

- [ ] **Step 1: Write the migration**

```sql
-- Migration 0008: remove the placeholder demo roster + retire the manual
-- add-teacher candidates flow. The roster is now synced from Sling.
--
-- Placeholder teachers were seeded with ids 1001..1010 ("Teacher A".."J").
-- Delete them (and their qualifications) ONLY when no proposal references them,
-- so any with real history are preserved (the Sling sync will deactivate those
-- instead). Idempotent: re-running deletes nothing further.

DELETE FROM teacher_qualifications
WHERE sling_user_id BETWEEN 1001 AND 1010
  AND sling_user_id NOT IN (SELECT sling_user_id FROM proposal_shifts);

DELETE FROM teachers
WHERE sling_user_id BETWEEN 1001 AND 1010
  AND sling_user_id NOT IN (SELECT sling_user_id FROM proposal_shifts);

-- The manual "add teacher from pull" candidates table is no longer used.
DROP TABLE IF EXISTS sling_candidates;
```

- [ ] **Step 2: Confirm the migration runner picks it up.** Read `src-tauri/src/migrations.rs` to confirm migrations are discovered by filename order (0001..0008). No code change expected — the runner globs the `migrations/` dir. If it uses an explicit list/array, add `0008_drop_demo_roster.sql` to it.

- [ ] **Step 3: Verify it compiles + applies.**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: builds. (The migration runs at app startup against the dev DB; a build is the compile-time check. If `migrations.rs` embeds SQL via `include_str!`/a list, the build fails until Step 2 is done — fix then.)

- [ ] **Step 4: Commit** (confirm branch first)

```bash
git rev-parse --abbrev-ref HEAD
git add src-tauri/migrations/0008_drop_demo_roster.sql src-tauri/src/migrations.rs
git commit -m "feat(roster): migration 0008 — purge demo teachers + drop sling_candidates"
```

---

## Task 2: Stop seeding the demo roster

**Files:** Modify `src-tauri/src/seed.rs`

- [ ] **Step 1: Read `src-tauri/src/seed.rs`** to find the function that inserts teachers/positions/qualifications (it early-returns when `teachers` is non-empty) and how it's called from `lib.rs` / `db.rs`.

- [ ] **Step 2: Make seeding a no-op.** Replace the body that inserts `TEACHERS`/`POSITIONS`/qualifications so it inserts nothing. Keep the function signature (so the caller compiles) but have it return early with a log line. Concretely, replace the insert loops with:

```rust
pub fn seed_if_empty(conn: &duckdb::Connection) -> anyhow::Result<()> {
    // Intentionally empty: the roster, positions, and qualifications are now
    // sourced from Sling (Settings → log in, set studio config, then
    // "Refresh from Sling" on the Teachers page, or run a month pull).
    // A fresh install starts with no teachers/positions on purpose — no
    // placeholder data that obscures whether Sling is actually connected.
    let _ = conn;
    eprintln!("[seed] roster/positions are Sling-sourced; nothing seeded");
    Ok(())
}
```

(Match the real function name/signature found in Step 1. Delete the now-unused `TEACHERS`/`POSITIONS`/`LEAD_*`/`TeacherSeed`/`PositionSeed` constants and structs to avoid dead-code warnings.)

- [ ] **Step 3: Verify compiles**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: builds, no dead-code warnings from removed seed constants.

- [ ] **Step 4: Commit** (confirm branch)

```bash
git add src-tauri/src/seed.rs
git commit -m "feat(roster): stop seeding placeholder roster/positions (empty start)"
```

---

## Task 3: `is_schedulable_teacher` helper (sling.rs)

**Files:** Modify `src-tauri/src/sling.rs`; tests in same file.

- [ ] **Step 1: Write the failing test** (inside `#[cfg(test)] mod tests`)

```rust
#[test]
fn is_schedulable_teacher_requires_active_home_and_qualified() {
    let mut schedulable = std::collections::HashSet::new();
    schedulable.insert(900i64); // a schedulable position group
    let home = 5i64;
    let mk = |active: bool, groups: Vec<i64>| SlingUser {
        id: 1, name: "T".into(), lastname: "X".into(), active, group_ids: groups,
    };
    // active + at home + qualified -> true
    assert!(is_schedulable_teacher(&mk(true, vec![5, 900]), home, &schedulable));
    // inactive -> false
    assert!(!is_schedulable_teacher(&mk(false, vec![5, 900]), home, &schedulable));
    // not at home location -> false
    assert!(!is_schedulable_teacher(&mk(true, vec![900]), home, &schedulable));
    // at home but not qualified for any schedulable position -> false
    assert!(!is_schedulable_teacher(&mk(true, vec![5, 777]), home, &schedulable));
}
```

(Confirm `SlingUser`'s field set when writing the literal — it is `{ id, name, lastname, active, group_ids }` per the struct in this file. Adjust the literal if fields differ.)

- [ ] **Step 2: Run → FAIL**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib is_schedulable_teacher`
Expected: cannot find function.

- [ ] **Step 3: Implement** (add to `sling.rs`)

```rust
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
```

- [ ] **Step 4: Run → PASS**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib`
Expected: all pass (existing + this one).

- [ ] **Step 5: Commit** (confirm branch)

```bash
git add src-tauri/src/sling.rs
git commit -m "feat(roster): is_schedulable_teacher predicate + test"
```

---

## Task 4: `sync_roster` + rewire the pull

**Files:** Modify `src-tauri/src/commands.rs`

This is the centerpiece. It extracts and upgrades the teacher/position/qual logic from `pull_month_from_sling` into a shared routine that auto-imports the full roster and deactivates departed entries.

- [ ] **Step 1: Add `RosterSyncSummary` + `sync_roster`** to `commands.rs` (place near `pull_month_from_sling`). `duckdb::Transaction` derefs to `Connection`, so callers pass `&tx`.

```rust
#[derive(serde::Serialize, Clone)]
pub struct RosterSyncSummary {
    pub teachers_active: i64,
    pub teachers_deactivated: i64,
    pub positions_active: i64,
    pub positions_deactivated: i64,
    pub qualifications: i64,
}

/// Reconcile the roster + positions + qualifications against Sling.
/// Sling is the source of truth: active home-location users qualified for a
/// schedulable position are imported; departed teachers and removed positions
/// are deactivated (never deleted — schedule history references them). App-only
/// fields (teacher caps/variety/notes; position duration/is_special/active) are
/// preserved across syncs. Must be called inside a transaction.
fn sync_roster(
    conn: &duckdb::Connection,
    users: &[crate::sling::SlingUser],
    groups: &[crate::sling::SlingGroup],
    cfg: &crate::sling::StudioConfig,
) -> Result<RosterSyncSummary, String> {
    use std::collections::HashSet;

    // ---- 1. Positions from Sling position-type groups ----
    let pos_groups: Vec<(i64, String)> = groups.iter()
        .filter(|g| g.kind == "position")
        .map(|g| (g.id, g.name.clone()))
        .collect();
    let sling_pos_ids: HashSet<i64> = pos_groups.iter().map(|(id, _)| *id).collect();

    for (id, name) in &pos_groups {
        let pid = *id as i32;
        let exists: i64 = conn.query_row(
            "SELECT count(*) FROM positions WHERE sling_position_id = ?",
            duckdb::params![pid], |r| r.get(0)).map_err(err)?;
        if exists > 0 {
            // Preserve duration_minutes / is_special / active (lead-managed).
            conn.execute("UPDATE positions SET class_name = ? WHERE sling_position_id = ?",
                duckdb::params![name, pid]).map_err(err)?;
        } else {
            conn.execute(
                "INSERT INTO positions (sling_position_id, class_name, duration_minutes, is_special, active)
                 VALUES (?, ?, 60, FALSE, TRUE)",
                duckdb::params![pid, name]).map_err(err)?;
        }
    }
    // Deactivate positions no longer present as Sling position-groups.
    let all_pos: Vec<i32> = {
        let mut s = conn.prepare("SELECT sling_position_id FROM positions").map_err(err)?;
        s.query_map([], |r| r.get(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
    };
    let mut positions_deactivated = 0i64;
    for pid in &all_pos {
        if !sling_pos_ids.contains(&(*pid as i64)) {
            conn.execute("UPDATE positions SET active = FALSE WHERE sling_position_id = ?",
                duckdb::params![pid]).map_err(err)?;
            positions_deactivated += 1;
        }
    }

    // ---- 2. Schedulable position set (active positions) ----
    let schedulable: HashSet<i64> = {
        let mut s = conn.prepare("SELECT sling_position_id FROM positions WHERE active = TRUE").map_err(err)?;
        s.query_map([], |r| r.get::<_, i32>(0)).map_err(err)?
            .collect::<Result<Vec<_>, _>>().map_err(err)?
            .into_iter().map(|p| p as i64).collect()
    };
    let positions_active = schedulable.len() as i64;

    // ---- 3. Teachers ----
    let location_names = crate::sling::location_name_by_id(groups);
    let mut imported: HashSet<i32> = HashSet::new();
    let mut teachers_active = 0i64;
    for u in users {
        if !crate::sling::is_schedulable_teacher(u, cfg.home_location_id, &schedulable) { continue; }
        let uid = u.id as i32;
        imported.insert(uid);
        teachers_active += 1;
        let display = format!("{} {}", u.name, u.lastname).trim().to_string();
        let locations = crate::sling::compute_locations(&u.group_ids, &location_names);
        let is_lead = u.id == cfg.acting_user_id;
        let exists: i64 = conn.query_row(
            "SELECT count(*) FROM teachers WHERE sling_user_id = ?",
            duckdb::params![uid], |r| r.get(0)).map_err(err)?;
        if exists > 0 {
            // Preserve weekly_target/weekly_max/variety_multiplier/ranking_weight/notes.
            conn.execute(
                "UPDATE teachers SET display_name = ?, locations = ?, active = TRUE, is_lead = ?
                 WHERE sling_user_id = ?",
                duckdb::params![display, locations, is_lead, uid]).map_err(err)?;
        } else {
            conn.execute(
                "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max,
                    is_lead, ranking_weight, variety_multiplier, active, locations)
                 VALUES (?, ?, 4, 5, ?, 1.0, 1.0, TRUE, ?)",
                duckdb::params![uid, display, is_lead, locations]).map_err(err)?;
        }
    }
    // Deactivate teachers not in the imported set.
    let all_teachers: Vec<i32> = {
        let mut s = conn.prepare("SELECT sling_user_id FROM teachers").map_err(err)?;
        s.query_map([], |r| r.get(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err)?
    };
    let mut teachers_deactivated = 0i64;
    for tid in &all_teachers {
        if !imported.contains(tid) {
            conn.execute("UPDATE teachers SET active = FALSE WHERE sling_user_id = ?",
                duckdb::params![tid]).map_err(err)?;
            teachers_deactivated += 1;
        }
    }

    // ---- 4. Qualifications (imported teachers × schedulable positions) ----
    let mut sling_pairs: HashSet<(i32, i32)> = HashSet::new();
    for u in users {
        let uid = u.id as i32;
        if !imported.contains(&uid) { continue; }
        for g in &u.group_ids {
            if schedulable.contains(g) {
                sling_pairs.insert((uid, *g as i32));
            }
        }
    }
    let existing: Vec<(i32, i32, bool)> = {
        let mut s = conn.prepare(
            "SELECT sling_user_id, sling_position_id, is_blocklisted FROM teacher_qualifications").map_err(err)?;
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(err)?
            .collect::<Result<_, _>>().map_err(err)?
    };
    for (uid, pid, blocked) in &existing {
        if *blocked { continue; }
        if !sling_pairs.contains(&(*uid, *pid)) {
            conn.execute("DELETE FROM teacher_qualifications WHERE sling_user_id = ? AND sling_position_id = ?",
                duckdb::params![uid, pid]).map_err(err)?;
        }
    }
    let mut qualifications = 0i64;
    for (uid, pid) in &sling_pairs {
        conn.execute(
            "INSERT INTO teacher_qualifications (sling_user_id, sling_position_id)
             VALUES (?, ?) ON CONFLICT DO NOTHING",
            duckdb::params![uid, pid]).map_err(err)?;
        qualifications += 1;
    }

    Ok(RosterSyncSummary { teachers_active, teachers_deactivated, positions_active, positions_deactivated, qualifications })
}
```

- [ ] **Step 2: Rewire `pull_month_from_sling`.** Read the current teacher/position/qual block (the section that builds `known_user_ids`/`known_position_ids`, deletes + refills `sling_candidates`, the per-user update/candidate loop, and the qualifications reconcile — roughly the block shown in the spec's context). Replace that entire block with a single call, keeping the availability-blocks and external-shifts logic that follows it unchanged:

```rust
    // Roster + positions + qualifications are reconciled from Sling here.
    let _roster = sync_roster(&tx, &payload.users, &payload.groups, &cfg)?;
```

Remove: the `DELETE FROM sling_candidates` + candidate INSERTs, the `new_users` accumulation, and the now-unused `known_user_ids`/`known_position_ids`/`position_group_ids`/`home_teacher_uids` locals **only if** they're no longer referenced by the remaining availability/external-shift code (some are — e.g. `known_user_ids` is used to filter availability blocks; if so, derive the active-teacher id set instead: replace `known_user_ids.contains(&uid)` checks in the availability/external loops with a freshly queried `SELECT sling_user_id FROM teachers` set after `sync_roster`). Read carefully and keep those loops working.

- [ ] **Step 3: Update the pull's return value.** `pull_month_from_sling` currently returns counts including `new_users`. Remove `new_users` from the returned struct (and stop building it). Keep the other counts. (The `PullResult`/return struct edit pairs with Task 7's TS change — change the Rust struct here; if the struct is defined in this file, drop the `new_users` field.)

- [ ] **Step 4: Verify compiles + tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib`
Expected: compiles; existing tests pass. (`add_teacher_from_pull`/`list_sling_candidates` may still exist — removed in Task 6 — so no error yet.)

- [ ] **Step 5: Commit** (confirm branch)

```bash
git add src-tauri/src/commands.rs
git commit -m "feat(roster): sync_roster routine; pull auto-syncs full roster from Sling"
```

---

## Task 5: `refresh_roster_from_sling` command

**Files:** Modify `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the command** (in `commands.rs`, near `discover_studio_config`). It fetches users + groups (not month-scoped) and runs `sync_roster` in a transaction. Reuse the token + studio-config guards from `pull_month_from_sling`.

```rust
#[tauri::command]
pub fn refresh_roster_from_sling(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
) -> Result<RosterSyncSummary, String> {
    let token_str = {
        let t = token.0.lock().map_err(err)?;
        t.clone().ok_or_else(|| "no Sling token — log in to Sling first".to_string())?
    };
    let cfg = {
        let conn = db.0.lock().map_err(err)?;
        load_studio_config(&conn)?
    };
    if cfg.org_id == 0 || cfg.home_location_id == 0 {
        return Err("Studio not configured — set your Sling org, acting-user, and location IDs in \
                    Settings → Studio configuration before refreshing the roster.".to_string());
    }
    // Fetch the roster inputs (reuses sling.rs helpers used by discover_studio).
    let users = crate::sling::fetch_users(&token_str)?;
    let groups = crate::sling::fetch_groups(&token_str)?;
    let mut conn = db.0.lock().map_err(err)?;
    let tx = conn.transaction().map_err(err)?;
    let summary = sync_roster(&tx, &users, &groups, &cfg)?;
    tx.commit().map_err(err)?;
    Ok(summary)
}
```

- [ ] **Step 2: Add `fetch_users` + `fetch_groups` to `sling.rs`** (extract the parsing the pull/discovery already do, so both the command and discovery can reuse them). Place near `fetch_calendar`:

```rust
/// GET /v1/users/concise → roster (with group memberships).
pub fn fetch_users(token: &str) -> Result<Vec<SlingUser>> {
    let doc = http_get(token, &format!("{BASE_URL}/users/concise"))?;
    Ok(doc.get("users").and_then(|v| v.as_array()).into_iter().flatten()
        .filter_map(|u| serde_json::from_value(u.clone()).ok()).collect())
}

/// GET /v1/groups → position + location groups.
pub fn fetch_groups(token: &str) -> Result<Vec<SlingGroup>> {
    let doc = http_get(token, &format!("{BASE_URL}/groups"))?;
    Ok(doc.as_array().ok_or_else(|| anyhow!("groups not array"))?
        .iter().filter_map(|g| serde_json::from_value(g.clone()).ok()).collect())
}
```

(Optional cleanup: have `discover_studio` and `pull_month` call these instead of inlining the same GET+parse. Not required for this task — only add the two functions.)

- [ ] **Step 3: Register** in `lib.rs` invoke handler — add `commands::refresh_roster_from_sling,`.

- [ ] **Step 4: Verify compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Finished, no errors.

- [ ] **Step 5: Commit** (confirm branch)

```bash
git add src-tauri/src/commands.rs src-tauri/src/sling.rs src-tauri/src/lib.rs
git commit -m "feat(roster): refresh_roster_from_sling command + fetch_users/fetch_groups"
```

---

## Task 6: `set_position_active` + retire candidate commands

**Files:** Modify `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `set_position_active`** in `commands.rs`:

```rust
#[tauri::command]
pub fn set_position_active(db: State<'_, Db>, sling_position_id: i64, active: bool) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    conn.execute("UPDATE positions SET active = ? WHERE sling_position_id = ?",
        duckdb::params![active, sling_position_id as i32]).map_err(err)?;
    Ok(())
}
```

- [ ] **Step 2: Remove the retired commands.** Delete the `add_teacher_from_pull` and `list_sling_candidates` command functions from `commands.rs` (and the `SlingCandidate` struct / `NewUserSummary` struct if now unused — grep to confirm no other references before deleting `NewUserSummary`; it was returned by the pull, removed in Task 4).

- [ ] **Step 3: Update `lib.rs` invoke handler** — remove `commands::add_teacher_from_pull,` and `commands::list_sling_candidates,`; add `commands::set_position_active,`.

- [ ] **Step 4: Verify compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Finished, no errors (no references to removed items remain in Rust).

- [ ] **Step 5: Commit** (confirm branch)

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(roster): set_position_active; retire add_teacher_from_pull/list_sling_candidates"
```

---

## Task 7: Frontend types + API

**Files:** Modify `src/types.ts`, `src/lib/api.ts`

- [ ] **Step 1: types.ts** — add:

```ts
export interface RosterSyncSummary {
  teachers_active: number;
  teachers_deactivated: number;
  positions_active: number;
  positions_deactivated: number;
  qualifications: number;
}
```

Remove the `new_users: NewUserSummary[]` field from `PullResult` (and delete the `NewUserSummary` and `SlingCandidate` interfaces if no longer referenced — grep first).

- [ ] **Step 2: api.ts** — in the `api` object: add

```ts
  refreshRosterFromSling: () => invoke<RosterSyncSummary>("refresh_roster_from_sling"),
  setPositionActive: (slingPositionId: number, active: boolean) =>
    invoke<void>("set_position_active", { slingPositionId, active }),
```

Remove `listSlingCandidates` and `addTeacherFromPull` methods, and their now-unused type imports. Add `RosterSyncSummary` to the `import type { … }` block.

- [ ] **Step 3: Verify type-check**

Run: `npm run build`
Expected: FAILS — `App.tsx` still references removed api methods / `new_users`. That's expected; Task 8 fixes the call sites. (If you want a green checkpoint, do Step 4 commit after Task 8 instead. To keep commits compiling, proceed to Task 8 before committing this — or temporarily leave the removals to Task 8. Recommended: fold the api removals into the commit at the end of Task 8.)

- [ ] **Step 4: Commit the additions only now** (the type + the two new api methods), and defer the removals to Task 8 so the tree stays buildable:

Add `RosterSyncSummary` type + `refreshRosterFromSling`/`setPositionActive` api, leave `listSlingCandidates`/`addTeacherFromPull`/`new_users` in place for now.

```bash
git add src/types.ts src/lib/api.ts
git commit -m "feat(roster): RosterSyncSummary type + refresh/setPositionActive api"
```

---

## Task 8: TeachersView + ProposalsView

**Files:** Modify `src/App.tsx`

- [ ] **Step 1: Read `TeachersView`, `AddTeacherCard`, `TeacherRow`, and the `ProposalsView` pull handler** to confirm anchors (the candidate state, `newUsersFromPull`, the `<AddTeacherCard …/>` render, the teachers list `.map`).

- [ ] **Step 2: TeachersView — add Refresh button + empty state, hide inactive, drop candidates.** Replace the candidate-loading effect and the `<AddTeacherCard …/>` render. Concretely:

  - Remove `const [candidates, setCandidates] = useState(...)` and the `api.listSlingCandidates()` call in the refresh effect.
  - Add:
    ```tsx
    const [syncing, setSyncing] = useState(false);
    const [syncMsg, setSyncMsg] = useState<string | null>(null);
    const onRefreshRoster = async () => {
      setSyncing(true); setSyncMsg(null); setError(null);
      try {
        const s = await api.refreshRosterFromSling();
        setSyncMsg(`Synced from Sling: ${s.teachers_active} teachers, ${s.positions_active} class types` +
          (s.teachers_deactivated ? `, ${s.teachers_deactivated} deactivated` : "") + ".");
        await refresh();
      } catch (e) {
        const msg = String(e);
        if (msg.includes("sling-401")) setSyncMsg("Sling token expired — log in again (Settings), then Refresh.");
        else if (msg.includes("not configured")) setSyncMsg("Set Studio configuration in Settings first, then Refresh.");
        else setSyncMsg(`Refresh failed: ${msg}`);
      } finally { setSyncing(false); }
    };
    ```
  - In the header row next to the `<h2>Teachers</h2>`, add: `<button className="btn-primary" onClick={onRefreshRoster} disabled={syncing}>{syncing ? "Refreshing…" : "Refresh from Sling"}</button>` and render `{syncMsg && <div className="muted">{syncMsg}</div>}`.
  - Filter the teacher list to active only: where it maps teachers, use `teachers.filter((t) => t.active).map(...)`.
  - Empty state: if `teachers.filter(t => t.active).length === 0`, render `<p className="muted">No teachers yet — log in to Sling, set Studio configuration in Settings, then click "Refresh from Sling".</p>` instead of the table.
  - Remove the `<AddTeacherCard candidates={addable} onAdded={refresh} />` render and the `addable` computation.

- [ ] **Step 3: Delete the now-unused `AddTeacherCard` function** and any `SlingCandidate` import.

- [ ] **Step 4: ProposalsView — remove the new-user banner.** Remove the `newUsersFromPull` state, its `setNewUsersFromPull(r.new_users)` assignment in the pull handler, and the JSX that renders the "new teacher detected" banner. (The pull result no longer has `new_users`.)

- [ ] **Step 5: Finish Task 7's removals** now that call sites are gone: in `api.ts` remove `listSlingCandidates`/`addTeacherFromPull`; in `types.ts` remove `NewUserSummary`/`SlingCandidate` if unreferenced.

- [ ] **Step 6: Verify build**

Run: `npm run build`
Expected: succeeds (no references to removed api/types remain).

- [ ] **Step 7: Commit** (confirm branch)

```bash
git add src/App.tsx src/lib/api.ts src/types.ts
git commit -m "feat(roster): Teachers Refresh-from-Sling + empty state; retire add-teacher/candidates UI"
```

---

## Task 9: PositionsView — schedulable toggle

**Files:** Modify `src/App.tsx` (`PositionsView`); confirm `Position` type has `active`.

- [ ] **Step 1: Confirm the `Position` TS type includes `active: boolean`.** Read `src/types.ts`'s `Position` interface; if `active` is missing, add it (the Rust `list_positions` already returns it — verify, and add to the query/struct if needed).

- [ ] **Step 2: Add an active toggle to `PositionsView`.** In the positions table row map, add a cell with a checkbox bound to `p.active` that calls the command and refreshes:

```tsx
<td>
  <input
    type="checkbox"
    checked={p.active}
    onChange={async (e) => {
      try { await api.setPositionActive(p.sling_position_id, e.target.checked); await refresh(); }
      catch (err) { setError(String(err)); }
    }}
  /> schedulable
</td>
```

Add a column header "Schedulable". Add a one-line caption: `<p className="muted">Uncheck non-class positions (e.g. Sales Rep) so they're excluded from the roster and scheduling.</p>`. Ensure `PositionsView` has a `refresh`/`setError` in scope (add `const [error,setError]=useState<string|null>(null)` + a `refresh` that re-fetches if not present).

- [ ] **Step 3: Verify build**

Run: `npm run build`
Expected: succeeds.

- [ ] **Step 4: Commit** (confirm branch)

```bash
git add src/App.tsx src/types.ts
git commit -m "feat(roster): positions schedulable toggle"
```

---

## Task 10: Algorithm verification (no code change expected)

**Files:** none expected — this is a verification task under the `schedule-algorithm` skill. Only touch `propose.py` if the verification reveals a gap, and if so reproduce the prior month first.

- [ ] **Step 1: Confirm the active-only candidate pool.** Read `generate_proposal` in `commands.rs` — confirm the teachers query feeding `propose.py` is `… FROM teachers WHERE active = TRUE`. Confirm `propose.py` builds `TEACHERS`/targets from the stdin `teachers` list (so inactive teachers are absent).

- [ ] **Step 2: Confirm unfillable slots are flagged, not silently dropped.** Read `propose.py`'s drop path (`dropped_slots`, the per-slot `reason`, and how a dropped/unassigned slot is emitted with `is_dropped`/`flag`). Confirm an unfillable slot is emitted with a reason and surfaced (it feeds the proposal's issues/flags).

- [ ] **Step 3: Reproduce + verify with a synthetic case.** Using the existing propose.py invocation path, run a generation where one historically-active teacher is now `active = false` (mark one inactive, regenerate the same month) and confirm: (a) that teacher gets zero assignments, (b) their former slots are either reassigned to an active eligible teacher or emitted unassigned-with-flag — never force-assigned, never silently gone. Capture the before/after load diff per the schedule-algorithm skill.

- [ ] **Step 4: Document the finding** in `docs/decisions/` (per the schedule-algorithm skill): note that inactive teachers are excluded via the active-only pool and unfillable slots are flagged — and whether any code change was needed.

```bash
git add docs/decisions/  # + any propose.py change if one was actually required
git commit -m "docs(roster): verify proposer excludes inactive teachers + flags unfillable slots"
```

---

## Manual end-to-end validation (after all tasks)

1. Fresh/empty app (or after migration): Teachers page shows the empty state.
2. Log in to Sling → Settings auto-detects + Save studio config → Teachers page → "Refresh from Sling" → roster + positions populate from Sling.
3. Positions page: uncheck a non-class position (e.g. Sales Rep) → re-Refresh → anyone qualified *only* for it drops out of the roster.
4. In Sling, deactivate/remove a teacher → Refresh → they leave the working roster (active list) but past proposals still show them; the Teachers list no longer offers them.
5. Generate a month where a now-inactive teacher historically taught a slot → that slot is reassigned to an active teacher or flagged unassigned for manual fix — never auto-given to the departed teacher.
6. Confirm the month pull also refreshes the roster (same `sync_roster`).
