# Claude Proposal Editor + Versioned Algorithm — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the spec at `docs/superpowers/specs/2026-07-06-claude-proposal-editor-design.md`: a Claude interaction that applies concrete edits to a month's proposal (teacher, unassign, format), self-analyzes for durable patterns, and mints versioned algorithm updates as rules-as-data or (gated) code drafts — plus the API-key/model prerequisites.

**Architecture:** Three change tiers (edits → rules → code). Rules and code versions live in an append-only `algorithm_versions` table; scripts in `<app_local_data>/algorithms/`. Two-step Claude calls (editor call never carries script source). Everything user-approved with fast Apply-all/Adopt paths.

**Tech Stack:** Rust (Tauri 2, duckdb 1.10504, ureq), Python 3 (propose.py), React + TS, vitest, cargo test.

## Global Constraints

- DuckDB rules (CLAUDE.md gotcha): no UNIQUE beyond PK, no incoming FKs on UPDATEd tables, never UPDATE a PK, compare-before-write. New tables here are append-only or PK-upsert-only.
- Never edit shipped migrations; new work goes in `src-tauri/migrations/0010_*.sql`, registered in `migrations.rs`, tested against a fresh in-memory DB.
- `scripts/propose.py` with empty/absent rules must produce **byte-identical JSON** to today (schedule-algorithm skill).
- No new top-level dependencies (ureq/serde/duckdb/chrono all suffice; no diff crate — the UI shows the full new script + stats instead of a line diff).
- Plain CSS, Barre & Bloom tokens, sentence-case copy, Lucide icons, no emoji.
- Model IDs: `claude-opus-4-8` (default), `claude-sonnet-4-6`, `claude-haiku-4-5`. Exact strings, no date suffixes.
- All Claude surfaces disabled without an API key, with the "Set your API key in Settings first" title.
- Commit after each task with the message given in the task.

---

### Task 1: Migration 0010 — `app_settings` + `algorithm_versions`

**Files:**
- Create: `src-tauri/migrations/0010_algorithm_versions.sql`
- Modify: `src-tauri/src/migrations.rs` (MIGRATIONS array + tests)
- Modify: `docs/data-model.md` (append two table sections; DDL below is canonical)

**Interfaces:**
- Produces: tables `app_settings(key, value, updated_at)` and `algorithm_versions(version, description, rules, script_file, created_by, claude_run_id, adopted_at)` used by Tasks 3/4/5.

- [ ] **Step 1: Write the migration**

```sql
-- Migration 0010: Claude proposal editor backing tables.
--
-- app_settings: tiny key-value store for user preferences that belong in
-- the DB (first key: 'claude_model'). PK-only, no extra indexes, no FKs —
-- INSERT OR REPLACE upserts are safe under the DuckDB rules (CLAUDE.md).
--
-- algorithm_versions: append-only history of adopted algorithm versions.
-- version 9 is the implicit baseline (shipped scripts/propose.py, empty
-- rules); adopted versions start at 10. rules is a FULL snapshot (not a
-- delta). script_file NULL = run the shipped baseline script; otherwise a
-- file name under <app_local_data>/algorithms/ (resolution also checks
-- algorithms/archive/). Rows are inserted on explicit user adoption and
-- never UPDATEd; "last used" is derived from proposals.algorithm_version.

CREATE TABLE IF NOT EXISTS app_settings (
    key        VARCHAR PRIMARY KEY,
    value      VARCHAR NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS algorithm_versions (
    version       INTEGER PRIMARY KEY,
    description   VARCHAR NOT NULL,
    rules         JSON NOT NULL,
    script_file   VARCHAR,
    created_by    VARCHAR NOT NULL,      -- 'claude' | 'user'
    claude_run_id BIGINT,                -- provenance into claude_runs (app-enforced)
    adopted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- [ ] **Step 2: Register in `migrations.rs`**

```rust
    Migration {
        version: 10,
        label: "claude editor: app_settings + algorithm_versions",
        sql: include_str!("../migrations/0010_algorithm_versions.sql"),
    },
```

- [ ] **Step 3: Add failing test to `migrations.rs` tests module**

```rust
    /// Migration 0010 tables exist with the append-only/upsert shapes the
    /// commands rely on.
    #[test]
    fn migration_0010_tables() {
        let conn = fresh_db();
        conn.execute(
            "INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-opus-4-8')",
            [],
        ).expect("app_settings upsert");
        conn.execute(
            "INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-haiku-4-5')",
            [],
        ).expect("app_settings re-upsert");
        let v: String = conn.query_row(
            "SELECT value FROM app_settings WHERE key = 'claude_model'", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "claude-haiku-4-5");

        conn.execute(
            "INSERT INTO algorithm_versions (version, description, rules, created_by)
             VALUES (10, 'v10 — test', '{}', 'user')",
            [],
        ).expect("insert version row");
        let (ver, script): (i32, Option<String>) = conn.query_row(
            "SELECT version, script_file FROM algorithm_versions ORDER BY version DESC LIMIT 1",
            [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(ver, 10);
        assert!(script.is_none());
    }
```

- [ ] **Step 4: Run** `cd src-tauri && cargo test --lib migrations` — expect the new test to pass (it fails before Steps 1–2 with "table does not exist").
- [ ] **Step 5: Update `docs/data-model.md`** — append `### app_settings` and `### algorithm_versions` sections with the SQL above and the header comments condensed to prose.
- [ ] **Step 6: Commit** `feat: migration 0010 — app_settings + algorithm_versions`

---

### Task 2: propose.py rules intake + v9 parity test

**Files:**
- Modify: `scripts/propose.py` (knob section ~line 115–160, PRIORITY loop ~line 239, JSON_OUT payload ~line 641)
- Create: `scripts/tests/test_propose_rules.py` (plain python script, exits non-zero on failure)
- Create: `scripts/tests/fixture_payload.json`

**Interfaces:**
- Consumes: stdin payload gains optional `"rules": {...}` (schema in spec) and `"version_label": "vN"`.
- Produces: JSON output `algorithm_version` echoes `version_label` (default `"v9"`). Rules populate: `TEACHER_CLASS_BLOCKLIST`, `TEACHER_SLOT_BLOCKLIST`, `PRIORITY_ENTRIES` (replaces PRIORITY_SLOTS/PRIORITY_UID), `JUNE_SLOT_CLASS_OVERRIDES`, `VARIETY_PENALTY_MULTIPLIER`, `VARIETY_PENALTY_PER_CLASS`, `SAT_TIME_SHIFTS`, `SUN_TIME_SHIFTS`.

- [ ] **Step 1: Build the fixture** — synthetic payload with 3 teachers, 2 positions, `history_events` containing ≥2 occurrences of two weekday slots (so `slot_rule` forms), empty `month_events`, `target_month` `2026-08`. Generate it with a throwaway script and check the emitted JSON runs through `propose.py --from-stdin --json-out --target-month 2026-08` producing ≥1 shift.
- [ ] **Step 2: Write the parity test (failing only if behavior diverges later)**

```python
#!/usr/bin/env python3
"""propose.py rules regression: empty rules == no rules, byte-identical."""
import json, subprocess, sys, copy, pathlib

HERE = pathlib.Path(__file__).parent
ROOT = HERE.parent.parent
payload = json.loads((HERE / "fixture_payload.json").read_text())

def run(p):
    out = subprocess.run(
        [sys.executable, "scripts/propose.py", "--json-out", "--from-stdin",
         "--target-month", p["target_month"]],
        input=json.dumps(p).encode(), cwd=ROOT, capture_output=True, check=True)
    return out.stdout

base = run(payload)
with_empty_rules = copy.deepcopy(payload)
with_empty_rules["rules"] = {}
assert run(with_empty_rules) == base, "empty rules must be byte-identical to no rules"

labeled = copy.deepcopy(payload)
labeled["version_label"] = "v10"
out = json.loads(run(labeled))
assert out["algorithm_version"] == "v10", out["algorithm_version"]

blocked = copy.deepcopy(payload)
first_shift = json.loads(base)["shifts"][0]
blocked["rules"] = {"teacher_class_blocklist": [
    {"sling_user_id": first_shift["sling_user_id"], "class_name": first_shift["class_name"], "reason": "test"}]}
out2 = json.loads(run(blocked))
same_slot = [s for s in out2["shifts"]
             if s["shift_date"] == first_shift["shift_date"] and s["start_time"] == first_shift["start_time"]]
assert all(s["sling_user_id"] != first_shift["sling_user_id"] for s in same_slot), \
    "blocklisted teacher must not keep the slot"

assert json.loads(base)["algorithm_version"] == "v9"
print("OK")
```

- [ ] **Step 3: Implement in propose.py.** After the knob defaults (below `VARIETY_PENALTY_PER_CLASS = 0.3`):

```python
# ============================================================
# Rules-as-data (algorithm_versions): populate the override knobs from the
# payload. Empty/absent rules leave every knob at its default — output must
# stay byte-identical to v9 (scripts/tests/test_propose_rules.py).
# ============================================================
_WD_IDX = {'Mon': 0, 'Tue': 1, 'Wed': 2, 'Thu': 3, 'Fri': 4, 'Sat': 5, 'Sun': 6}
_rules = _payload.get('rules') or {}
VERSION_LABEL = _payload.get('version_label') or 'v9'
for _r in _rules.get('teacher_class_blocklist') or []:
    TEACHER_CLASS_BLOCKLIST.setdefault(_r['sling_user_id'], set()).add(_r['class_name'])
for _r in _rules.get('teacher_slot_blocklist') or []:
    TEACHER_SLOT_BLOCKLIST.setdefault(_r['sling_user_id'], set()).add((_WD_IDX[_r['weekday']], _r['time']))
PRIORITY_ENTRIES = [(_WD_IDX[_r['weekday']], _r['time'], _r['sling_user_id'])
                    for _r in _rules.get('priority_slots') or []]
for _r in _rules.get('slot_class_overrides') or []:
    JUNE_SLOT_CLASS_OVERRIDES[(_WD_IDX[_r['weekday']], _r['time'])] = _r['class_name']
for _uid, _mult in (_rules.get('variety_penalty_multiplier') or {}).items():
    VARIETY_PENALTY_MULTIPLIER[int(_uid)] = float(_mult)
if 'variety_penalty_per_class' in _rules:
    VARIETY_PENALTY_PER_CLASS = float(_rules['variety_penalty_per_class'])
SAT_TIME_SHIFTS.update(_rules.get('sat_time_shifts') or {})
SUN_TIME_SHIFTS.update(_rules.get('sun_time_shifts') or {})
```

Replace the PRIORITY seeding loop (and delete `PRIORITY_SLOTS` / `PRIORITY_UID` definitions):

```python
# Priority seeding (weight a teacher up at specific slots; from rules)
for (wd, st, uid) in PRIORITY_ENTRIES:
    for cls in slot_classes.get((wd, st), {}):
        if not teacher_qualified(uid, cls): continue
        slot_teachers[(wd, st, cls)][uid] = max(slot_teachers[(wd, st, cls)].get(uid, 0), 3)
```

In the JSON_OUT payload change `'algorithm_version': 'v9'` → `'algorithm_version': VERSION_LABEL`.

- [ ] **Step 4: Run** `python3 scripts/tests/test_propose_rules.py` — expect `OK`.
- [ ] **Step 5: Commit** `feat: propose.py consumes versioned rules from the payload (v9 parity kept)`

---

### Task 3: Anthropic key → Stronghold; app-settings + model commands

**Files:**
- Modify: `src-tauri/src/secrets.rs` (add `pub const KEY_ANTHROPIC: &[u8] = b"anthropic_key";`)
- Modify: `src-tauri/src/commands.rs` (`set_anthropic_key`, `has_anthropic_key`, new `get_app_setting`/`set_app_setting`, helpers)
- Modify: `src-tauri/src/lib.rs` (preload key at startup; register commands)
- Modify: `src/screens/SettingsScreen.tsx` (key copy + model select), `src/lib/api.ts`, `src/types.ts`, `src/lib/devMock.ts`

**Interfaces:**
- Produces (Rust): `fn claude_model(conn: &duckdb::Connection) -> String` (reads `app_settings['claude_model']`, falls back to `"claude-opus-4-8"`, rejects unknown ids by falling back); commands `get_app_setting(key) -> Option<String>`, `set_app_setting(key, value)`.
- Produces (TS): `api.getAppSetting(key)`, `api.setAppSetting(key, value)`.
- Model allowlist constant shared with Task 7: `pub const CLAUDE_MODELS: &[&str] = &["claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5"];`

- [ ] **Step 1: Failing test in commands.rs tests module**

```rust
    #[test]
    fn claude_model_setting_roundtrip_and_fallback() {
        let conn = conn_with_schema();
        assert_eq!(claude_model(&conn), "claude-opus-4-8"); // unset → default
        conn.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-haiku-4-5')", []).unwrap();
        assert_eq!(claude_model(&conn), "claude-haiku-4-5");
        conn.execute("INSERT OR REPLACE INTO app_settings (key, value) VALUES ('claude_model', 'claude-9000')", []).unwrap();
        assert_eq!(claude_model(&conn), "claude-opus-4-8"); // unknown → default
    }
```

- [ ] **Step 2: Implement.** `claude_model` helper + commands:

```rust
pub const CLAUDE_MODELS: &[&str] = &["claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5"];
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-opus-4-8";

fn claude_model(conn: &duckdb::Connection) -> String {
    let stored: Option<String> = conn
        .query_row("SELECT value FROM app_settings WHERE key = 'claude_model'", [], |r| r.get(0))
        .ok();
    match stored {
        Some(m) if CLAUDE_MODELS.contains(&m.as_str()) => m,
        _ => DEFAULT_CLAUDE_MODEL.to_string(),
    }
}

#[tauri::command]
pub fn get_app_setting(db: State<'_, Db>, key: String) -> Result<Option<String>, String> {
    let conn = db.0.lock().map_err(err)?;
    Ok(conn.query_row("SELECT value FROM app_settings WHERE key = ?", duckdb::params![key], |r| r.get(0)).ok())
}

#[tauri::command]
pub fn set_app_setting(db: State<'_, Db>, key: String, value: String) -> Result<(), String> {
    let conn = db.0.lock().map_err(err)?;
    conn.execute("INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?, ?, now())",
        duckdb::params![key, value]).map_err(err)?;
    Ok(())
}
```

`set_anthropic_key` gains `secrets: State<'_, crate::secrets::Secrets>` and mirrors `set_sling_token` (empty → `remove(KEY_ANTHROPIC)`, else `set`). In `lib.rs` startup, preload `KEY_ANTHROPIC` from the vault into `AnthropicKey` the same way `initial_token` is loaded for Sling.

- [ ] **Step 3: Frontend.** Settings → Anthropic card: status copy becomes "Stored in the OS keychain (Stronghold) — survives restarts."; add a `Field` labelled "Model" with a `<select>` of the three ids and descriptions ("Most capable — ~13¢ per interaction", "Balanced — ~7¢", "Cheapest — ~2–3¢"), value loaded from `api.getAppSetting("claude_model")` (default `claude-opus-4-8`), saved on change via `api.setAppSetting`. devMock: back `get_app_setting`/`set_app_setting` with an in-memory map.
- [ ] **Step 4: Run** `cargo test --lib commands` and `npx vitest run`; expect green. `cargo check` clean.
- [ ] **Step 5: Commit** `feat: Stronghold-backed Anthropic key + user-selectable Claude model`

---

### Task 4: `algorithm.rs` — rules validation, version store, script files, sweep

**Files:**
- Create: `src-tauri/src/algorithm.rs`
- Modify: `src-tauri/src/lib.rs` (mod + startup sweep + register commands)
- Modify: `src-tauri/src/commands.rs` (thin command wrappers)

**Interfaces (produces — used by Tasks 5/7/8/10):**

```rust
// algorithm.rs
pub const BASELINE_VERSION: i32 = 9;

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(default)] pub teacher_class_blocklist: Vec<ClassBlock>,
    #[serde(default)] pub teacher_slot_blocklist: Vec<SlotBlock>,
    #[serde(default)] pub priority_slots: Vec<PrioritySlot>,
    #[serde(default)] pub slot_class_overrides: Vec<SlotClassOverride>,
    #[serde(default)] pub variety_penalty_multiplier: std::collections::HashMap<String, f64>,
    #[serde(default)] pub variety_penalty_per_class: Option<f64>,
    #[serde(default)] pub sat_time_shifts: std::collections::HashMap<String, String>,
    #[serde(default)] pub sun_time_shifts: std::collections::HashMap<String, String>,
}
// ClassBlock { sling_user_id: i32, class_name: String, #[serde(default)] reason: String }
// SlotBlock  { sling_user_id: i32, weekday: String, time: String, #[serde(default)] reason: String }
// PrioritySlot { sling_user_id: i32, weekday: String, time: String }
// SlotClassOverride { weekday: String, time: String, class_name: String }

pub fn validate_rules(raw: &serde_json::Value) -> Result<Rules, String>; // deny-unknown + weekday name check
#[derive(Serialize, Clone)]
pub struct AlgorithmVersion { pub version: i32, pub description: String, pub rules: serde_json::Value,
    pub script_file: Option<String>, pub created_by: String, pub adopted_at: String,
    pub last_used_month: Option<String>, pub script_archived: bool, pub script_missing: bool }
pub fn active_version(conn) -> Result<Option<AlgorithmVersion>, String>;   // max(version) row
pub fn list_versions(conn, algo_dir: &Path) -> Result<Vec<AlgorithmVersion>, String>;
pub fn adopt_version(conn, algo_dir, description, rules_raw, script_content: Option<String>, claude_run_id: Option<i64>, created_by: &str) -> Result<i32, String>;
pub fn resolve_script(algo_dir: &Path, v: &AlgorithmVersion, project_root: &Path) -> Result<PathBuf, String>; // NULL→root/scripts/propose.py; else algorithms/{f} → archive/{f} → Err
pub fn archive_sweep(conn, algo_dir) -> Result<Vec<String>, String>; // move files >3 versions behind active AND (never used || last used >3 months ago)
pub fn algorithms_dir(app: &tauri::AppHandle) -> Result<PathBuf, String>; // <app_local_data>/algorithms, created
```

`last_used_month` = `SELECT max(target_month) FROM proposals WHERE algorithm_version = 'v{N}'`.
`adopt_version`: version = `max(SELECT max(version), 9) + 1`; script file name `propose_v{N}.py`; write file **before** the INSERT; `created_by` = `'claude'` when `claude_run_id` is Some, else `'user'`.

- [ ] **Step 1: Failing tests** (in `algorithm.rs` `#[cfg(test)]`, fresh in-memory DB via `crate::migrations::run`, `std::env::temp_dir()` scratch dirs):
  - `validate_rules_rejects_unknown_keys` — `{"hard_assignments": []}` → Err; `{}` → Ok(default); bad weekday `"Sax"` → Err.
  - `adopt_assigns_sequential_versions` — first adopt → 10, second → 11; row fields round-trip; script content lands in `algorithms/propose_v11.py`.
  - `resolve_script_falls_back_to_archive_then_errors` — file in `algorithms/` resolves; moved to `archive/` still resolves; deleted → Err mentioning "adopt a newer version".
  - `archive_sweep_only_old_and_unused` — versions 10..14 with script files; proposal rows give 13/14 recent `last_used`; sweep moves only v10's file (>3 behind AND unused-3-months, using proposal `generated_at` inserted via SQL with `now() - INTERVAL 120 DAYS` for old use).
- [ ] **Step 2: Implement** module + thin `#[tauri::command]` wrappers in commands.rs: `list_algorithm_versions(app, db)`, `adopt_algorithm_version(app, db, description, rules, script_content, claude_run_id)`, `delete_algorithm_script(app, db, version)` (refuses the active version; deletes from both dirs). Startup (lib.rs setup, after migrations): `let moved = algorithm::archive_sweep(&conn, &algorithm::algorithms_dir(app.handle())?)?;` log each move.
- [ ] **Step 3: Run** `cargo test --lib algorithm` → green; full `cargo test` → green.
- [ ] **Step 4: Commit** `feat: algorithm version store (rules validation, script files, archive sweep)`

---

### Task 5: generate_proposal uses the active version

**Files:**
- Modify: `src-tauri/src/commands.rs` (`generate_proposal`; extract `build_propose_payload`)

**Interfaces:**
- Produces: `fn build_propose_payload(conn: &duckdb::Connection, target_month: &str) -> Result<serde_json::Value, String>` — the existing payload construction extracted verbatim (teachers, users_with_groups, history_events, month_events, home_location_id, incl. the no-history error). Reused by Task 8's validator.
- `generate_proposal` gains `app: tauri::AppHandle` as its first parameter (frontend call unchanged — Tauri injects it).

- [ ] **Step 1: Extract `build_propose_payload`** (pure refactor; `cargo test` stays green).
- [ ] **Step 2: Wire the active version.** Before spawning: load `active_version(&conn)?`; if Some(v): insert `payload["rules"] = v.rules`, `payload["version_label"] = format!("v{}", v.version)`, and resolve the script path via `resolve_script(...)`; if None: spawn the baseline `scripts/propose.py` with no extra keys (byte-identical payload to today). Spawn uses the resolved absolute path: `.args([script_path.to_str().unwrap(), "--json-out", "--from-stdin", "--target-month", &target_month])` with cwd still `project_root`.
- [ ] **Step 3: Test** — commands.rs test: adopt a v10 with a `slot_class_overrides` rule via `adopt_version`, then assert `active_version` roundtrips and `build_propose_payload` errors without history (existing behavior) — the full spawn path is exercised in visual QA (Task 12) since it needs python.
- [ ] **Step 4: Run** `cargo test`; commit `feat: generate_proposal runs the adopted algorithm version (rules + script)`

---

### Task 6: Format edits — `edit_proposal_shift_position` + edit-history names

**Files:**
- Modify: `src-tauri/src/commands.rs` (new command; extend `list_edits_for_proposal` query)
- Modify: `src/types.ts` (`EditRow` gains `old_class_name`/`new_class_name`), `src/lib/api.ts`, `src/lib/devMock.ts`

**Interfaces:**
- Produces: `edit_proposal_shift_position(proposal_shift_id: i64, new_position_id: i32, reason: Option<String>)`. Records edits row `field='sling_position_id'`, old/new = position ids as strings. Recomputes `end_time = start_time + positions.duration_minutes`. Rejects co-teach rows (`"co-teach editing is not yet supported"`) and unchanged position (`"class type unchanged"`) and unknown/inactive positions.
- `EditRow` gains `old_class_name: Option<String>`, `new_class_name: Option<String>` (LEFT JOIN positions on `CAST(p.sling_position_id AS VARCHAR) = e.old_value/new_value` **only when `e.field='sling_position_id'`** — join condition includes the field check so teacher edits don't cross-match).

- [ ] **Step 1: Failing test**

```rust
    #[test]
    fn edit_position_recomputes_end_time_and_audits() {
        let mut conn = conn_with_schema(); // has positions 29470407 (Classic) + 29470408 (Empower, 45 min via UPDATE below)
        conn.execute("UPDATE positions SET duration_minutes = 45 WHERE sling_position_id = 29470408", []).unwrap();
        let sid: i64 = conn.query_row("SELECT min(id) FROM proposal_shifts", [], |r| r.get(0)).unwrap();
        edit_position_impl(&mut conn, sid, 29470408, Some("format swap".into())).expect("edit ok");
        let (pid, end): (i32, String) = conn.query_row(
            "SELECT sling_position_id, end_time FROM proposal_shifts WHERE id = ?",
            duckdb::params![sid], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(pid, 29470408);
        assert_eq!(end, "09:45"); // 09:00 + 45min
        let field: String = conn.query_row(
            "SELECT field FROM edits WHERE proposal_shift_id = ? ORDER BY id DESC LIMIT 1",
            duckdb::params![sid], |r| r.get(0)).unwrap();
        assert_eq!(field, "sling_position_id");
        // same position again → error
        assert!(edit_position_impl(&mut conn, sid, 29470408, None).is_err());
    }
```

- [ ] **Step 2: Implement** `edit_position_impl(conn, id, new_pid, reason)` (transaction: read shift row incl. `is_coteach`, `start_time`, `sling_position_id`; guards; read `duration_minutes` of new position; compute end via an `add_minutes` helper on "HH:MM"; INSERT edits; UPDATE proposal_shifts SET sling_position_id, end_time; commit; CHECKPOINT) + the `#[tauri::command]` wrapper; extend the edits query + `EditRow` struct + TS mirror; devMock handler mutates the shift and appends an edit.
- [ ] **Step 3: Run** `cargo test --lib commands`, `npx vitest run`; commit `feat: change a slot's class format (audited, end time recomputed)`

---

### Task 7: `editor.rs` — the Claude editor call + `claude_edit_proposal`

**Files:**
- Create: `src-tauri/src/editor.rs`
- Create: `prompts/proposal-editor.md`
- Modify: `src-tauri/src/review.rs` (`run_review(api_key, model, payload)`; per-model pricing table shared via `pub fn model_prices(model: &str) -> (f64, f64, f64, f64)`)
- Modify: `src-tauri/src/commands.rs` (`review_proposal` passes `claude_model(&conn)`; new `claude_edit_proposal`), `src-tauri/src/lib.rs` (mod + register)

**Interfaces:**
- Produces (Rust):

```rust
// editor.rs
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProposedEdit {
    pub proposal_shift_id: i64,
    pub action: String,                    // "reassign" | "unassign" | "change_format"
    #[serde(default)] pub new_user_id: Option<i32>,
    #[serde(default)] pub new_class_name: Option<String>,
    pub rationale: String,
    #[serde(default = "default_true")] pub valid: bool,          // set app-side
    #[serde(default)] pub validation_note: Option<String>,        // set app-side
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RulesetProposal { pub description: String, pub rules: serde_json::Value }
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NeedsCodeChange { pub rationale: String }
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EditorPayload {
    pub summary: String,
    #[serde(default)] pub edits: Vec<ProposedEdit>,
    #[serde(default)] pub ruleset_proposal: Option<RulesetProposal>,
    #[serde(default)] pub needs_code_change: Option<NeedsCodeChange>,
}
pub fn run_editor(api_key: &str, model: &str, user_payload: &serde_json::Value) -> anyhow::Result<(EditorPayload, crate::review::ApiCall-like accounting)>;
pub fn editor_system_prompt(project_root: Option<&std::path::Path>) -> String; // prompts/proposal-editor.md if readable, else INLINE_PROMPT
```

- Produces (command): `claude_edit_proposal(proposal_id: i64, instruction: String) -> ClaudeEditResult { run_id: i64, summary, edits: Vec<ProposedEdit>, ruleset_proposal, needs_code_change, model, cost_usd, duration_ms }`. Persisted to `claude_runs` like reviews (same INSERT).
- Model pricing (USD/MTok, in `model_prices`): opus-4-8 `(5.0, 25.0, 6.25, 0.50)`, sonnet-4-6 `(3.0, 15.0, 3.75, 0.30)`, haiku-4-5 `(1.0, 5.0, 1.25, 0.10)` as `(input, output, cache_write, cache_read)`; unknown model → opus prices.

- [ ] **Step 1: Write `prompts/proposal-editor.md`** (this exact text; the inline Rust constant is a copy):

```markdown
You are the scheduling assistant for a barre studio's monthly class proposal.
The user gives you an instruction; you return concrete, minimal changes as JSON.

Input JSON contains: proposal (id, target_month, shifts — each with its
proposal_shift_id, date, start/end, class_name, teacher and ids), roster
(teachers with weekly target/max caps), qualifications (teacher × class),
availability_blocks (these are BLOCKED times — the teacher is UNAVAILABLE),
open_issues, edit_history, active_rules (the algorithm's standing rules),
and instruction.

Respond with ONLY valid JSON, no markdown fences:
{
  "summary": "one or two sentences describing what you changed and why",
  "edits": [
    {
      "proposal_shift_id": 123,
      "action": "reassign" | "unassign" | "change_format",
      "new_user_id": 456,          // reassign only
      "new_class_name": "Classic", // change_format only
      "rationale": "one line"
    }
  ],
  "ruleset_proposal": null,
  "needs_code_change": null
}

Rules for edits:
- Reference only proposal_shift_id values that exist in the input. Never invent slots.
- Respect qualifications, weekly caps, and availability blocks unless the
  instruction explicitly overrides them; if you must break one, say so in the rationale.
- Prefer the fewest edits that satisfy the instruction. Zero edits with an
  explanatory summary is a valid answer.
- unassign drops the class from the schedule (it will show as dropped).

Escalation tiers — always prefer the lowest tier that satisfies the instruction:
1. Proposal edits (above) — one-off changes to this month.
2. ruleset_proposal — ONLY when the instruction or the edit history shows a
   RECURRING pattern worth making permanent (e.g. the same teacher/class swap
   corrected repeatedly). Shape:
   {"description": "v-next — <what changed, human words>",
    "rules": { ...the FULL new rule set: active_rules with your change applied... }}
   Allowed rule keys: teacher_class_blocklist, teacher_slot_blocklist,
   priority_slots, slot_class_overrides, variety_penalty_multiplier,
   variety_penalty_per_class, sat_time_shifts, sun_time_shifts. Weekdays are
   "Mon".."Sun"; times "HH:MM"; teachers by sling_user_id.
3. needs_code_change — ONLY when the desired behavior cannot be expressed in
   those rule keys (new ranking logic, new constraint types). Shape:
   {"rationale": "why the rule keys above cannot express this"}. Do NOT write code.
Propose at most one of ruleset_proposal / needs_code_change per response, and
only when genuinely warranted — routine edits should leave both null.
```

- [ ] **Step 2: Failing tests** (editor.rs): `parses_editor_response_with_all_sections` (serde roundtrip of a full JSON sample), `system_prompt_falls_back_inline` (nonexistent root → inline text contains "Escalation tiers"), plus in review.rs a `model_prices_known_and_fallback` test.
- [ ] **Step 3: Implement.** `run_editor` mirrors `run_review` (same ureq/caching/`extract_json` shape — share via a small `fn call_anthropic(api_key, model, system, user_text, max_tokens) -> (raw_output, usage, duration)` in review.rs; MAX_TOKENS 8192 for the editor). `claude_edit_proposal` builds the payload: shifts WITH ids (query like `get_proposal`'s), roster, qualified pairs, availability blocks (month), open issues (recompute server-side is redundant — frontend computes; instead include the same inputs and let the model see blocks/caps; include `edit_history` like review), `active_rules` (from `active_version`, `{}` when none), `instruction`. Validate each returned edit against the DB (shift exists & not coteach & not the same teacher for reassign; teacher exists+active for reassign; class maps to an active position for change_format) — set `valid`/`validation_note` instead of dropping. Persist run to `claude_runs`. `review_proposal` switches to `claude_model(&conn)` + `model_prices`.
- [ ] **Step 4: Run** `cargo test`; commit `feat: Claude proposal editor call (edits + rule/code escalation), model-aware pricing`

---

### Task 8: Code drafts — `claude_draft_code_change` + `validate_code_draft`

**Files:**
- Modify: `src-tauri/src/editor.rs` (draft call), `src-tauri/src/commands.rs` (two commands), `src-tauri/src/lib.rs` (register)

**Interfaces:**
- `claude_draft_code_change(proposal_id, instruction, rationale) -> CodeDraft { run_id, description, script }`. Second call includes the **current active script source** (resolved via Task 4) in the user payload; system prompt addendum (inline in editor.rs): "Return ONLY JSON {\"description\": \"v-next — ...\", \"script\": \"<the complete new python script>\"}. The script must keep the same CLI (--json-out --from-stdin --target-month), the same stdin payload schema including rules, and the same output JSON schema; echo version_label as algorithm_version."
- `validate_code_draft(script_content: String) -> DraftValidation { ok: bool, error: Option<String>, shift_count: i64, changed_assignments: i64, added_slots: i64, removed_slots: i64, month: String }`:
  1. month = most recent `proposals.target_month`; its current proposal's shifts loaded as baseline.
  2. payload = `build_propose_payload(conn, month)` + active rules + `version_label` "candidate".
  3. write script to `<algorithms>/candidate_draft.py`, spawn it exactly like generate does, 120s wait; non-zero exit or unparseable `ProposeOutput` → `ok:false` with stderr tail.
  4. diff by slot key `(shift_date, start_time, sling_position_id)`: `changed_assignments` = same key, different `sling_user_id`; `added_slots`/`removed_slots` = key set difference vs baseline.
- Frontend "Adopt as vN" for code drafts calls `adopt_algorithm_version` with `script_content` + carried-forward rules; the UI enables it only after a `DraftValidation.ok`.

- [ ] **Step 1: Failing test** — `draft_validation_diff_counts` in commands.rs tests: build two shift vectors in-memory and test the extracted pure diff fn `fn diff_schedules(baseline: &[(String,String,i32,Option<i32>)], candidate: &[(String,String,i32,Option<i32>)]) -> (i64,i64,i64)`; cases: identical → (0,0,0); one reassign → (1,0,0); one extra/one missing slot → (0,1,1).
- [ ] **Step 2: Implement** the pure diff fn + both commands (spawn shares a helper with generate: `fn spawn_propose(script: &Path, project_root: &Path, payload: &Value, target_month: &str) -> Result<ProposeOutput, String>` extracted in this task and reused by `generate_proposal`).
- [ ] **Step 3: Run** `cargo test`; commit `feat: Claude code drafts with previous-month validation gate`

---

### Task 9: Frontend plumbing — types, api, devMock

**Files:**
- Modify: `src/types.ts`, `src/lib/api.ts`, `src/lib/devMock.ts`

**Interfaces (TS mirrors of Rust):**

```ts
export interface ProposedEdit { proposal_shift_id: number; action: "reassign" | "unassign" | "change_format";
  new_user_id?: number | null; new_class_name?: string | null; rationale: string;
  valid: boolean; validation_note?: string | null; }
export interface RulesetProposal { description: string; rules: Record<string, unknown>; }
export interface ClaudeEditResult { run_id: number; summary: string; edits: ProposedEdit[];
  ruleset_proposal: RulesetProposal | null; needs_code_change: { rationale: string } | null;
  model: string; cost_usd: number; duration_ms: number; }
export interface AlgorithmVersion { version: number; description: string; rules: Record<string, unknown>;
  script_file: string | null; created_by: string; adopted_at: string;
  last_used_month: string | null; script_archived: boolean; script_missing: boolean; }
export interface CodeDraft { run_id: number; description: string; script: string; }
export interface DraftValidation { ok: boolean; error: string | null; shift_count: number;
  changed_assignments: number; added_slots: number; removed_slots: number; month: string; }
```

api.ts adds: `claudeEditProposal(proposalId, instruction)`, `claudeDraftCodeChange(proposalId, instruction, rationale)`, `validateCodeDraft(script)`, `listAlgorithmVersions()`, `adoptAlgorithmVersion(description, rules, scriptContent?, claudeRunId?)`, `deleteAlgorithmScript(version)`, `editProposalShiftPosition(proposalShiftId, newPositionId, reason)`, `getAppSetting(key)`, `setAppSetting(key, value)` — all thin `invoke` wrappers with camelCase arg keys matching the Rust parameter names.

devMock adds handlers: `claude_edit_proposal` returns a canned result after ~1.2s (two valid edits against real mock shift ids — one reassign, one change_format — plus a ruleset_proposal adding Casey×Reform to the blocklist); `claude_draft_code_change` returns a stub script; `validate_code_draft` returns `{ok:true, changed_assignments:3, ...}`; `list_algorithm_versions` from an in-memory array seeded with a v10; `adopt_algorithm_version` appends; `edit_proposal_shift_position` mutates the mock shift (`class_name`, end time) and appends an edit row.

- [ ] Steps: implement, `npx vitest run` + `npx tsc -b` green, commit `feat: frontend plumbing for the Claude editor (types, api, dev mock)`

---

### Task 10: Claude tab rework

**Files:**
- Create: `src/components/claude/ClaudeEditorPanel.tsx` (instruction box + results), `src/components/claude/EditChecklist.tsx`, `src/components/claude/VersionProposalCard.tsx`, `src/components/claude/AlgorithmCard.tsx`
- Modify: `src/screens/ProposalsScreen.tsx` (tab renamed `claude`; composition), `src/styles.css` (small additions: `.bk-edit-row`, `.bk-edit-row.applied`, `.bk-code-scroll`)

**Behavior (complete spec — implement exactly):**
- Tab order: `["calendar", "list", "edits", "claude"]`; the claude tab renders, top to bottom: `ClaudeEditorPanel`, existing `ClaudeReviewSection`, `AlgorithmCard`.
- `ClaudeEditorPanel`: textarea (Field "Ask Claude to adjust this proposal", placeholder `e.g. "Give Morgan more Saturday classes" or "resolve the open conflicts"`), Send (`btn-primary`, Sparkles icon, disabled when empty/no key/running) + ghost shortcut "Resolve open conflicts" that sets the instruction to `Resolve the open conflicts in this proposal.` and sends. While running: `LoadingBlock label="Asking Claude…"`. On result: summary paragraph, cost line (`{model} · ${cost} · {s}s`), then:
  - `EditChecklist`: one row per edit — checkbox (default checked when `valid`), slot description (date/time/class from the proposal detail, looked up by `proposal_shift_id`), action text ("→ Kayla Moore" / "unassign" / "format → Classic"), rationale (muted), invalid rows disabled with `validation_note`. Buttons: "Apply selected (N)" (`btn-primary`) and per-row Apply. Applying loops `api.editProposalShiftTeacher` (reassign→`new_user_id`, unassign→`null`, reason `claude: {rationale}`) or `api.editProposalShiftPosition` (change_format → map `new_class_name` to `sling_position_id` via the positions list), marks rows applied (strikethrough, check), then `onProposalChanged()`.
  - `VersionProposalCard` when `ruleset_proposal`: description, `<pre>` of the rules JSON, "Adopt as v{next}" → `api.adoptAlgorithmVersion(description, rules, undefined, run_id)` → refresh AlgorithmCard + success line. Dismiss = plain close.
  - needs_code_change card: rationale + "Draft code change" (`btn-primary`) → `api.claudeDraftCodeChange(...)` → shows `CodeDraft` card: description, "Validate against {month}" button → `api.validateCodeDraft(script)` → stats line (`ok`: "Ran clean on {month}: N shifts, M assignments differ, +A/−R slots" / error tail) , collapsible `<pre class="bk-code-scroll">` with the script, and "Adopt as v{next}" enabled only after `ok` (passes `scriptContent`).
- `AlgorithmCard`: "Algorithm" heading; active version line ("v10 — Casey off Reform · adopted 2026-07-06 · last used 2026-08"); expandable rule list (render each rule array human-readably, e.g. `Casey Diaz — never Reform (reason)`); version history table (version, description, created_by, adopted, last used, script badge [baseline/file/archived/missing]) with per-row Delete script (`btn-ghost btn-sm`, hidden for active + baseline rows, confirm via one re-click "Really delete?").
- Empty-key state: the whole editor panel replaced by muted "Set your Anthropic API key in Settings to use the editor." (reuse `hasAnthropicKey`).

- [ ] Steps: implement, `npx tsc -b` green, visual pass against devMock (screenshot claude tab: idle, result-with-everything, applied state), commit `feat: claude tab — instruction box, edit checklist, version cards, algorithm card`

---

### Task 11: Day editor format picker

**Files:**
- Modify: `src/components/calendar/DayEditorPanel.tsx` (add "Change format" beside "Change teacher"), `src/screens/ProposalsScreen.tsx` (pass `positions` down via CalendarView), `src/components/calendar/CalendarView.tsx` (prop pass-through)

**Behavior:** a second ghost button `Shapes` icon "Change format" per slot toggles a picker listing active schedulable positions as `ClassChip size="md"` buttons (current one highlighted, click = no-op close); selecting calls `api.editProposalShiftPosition(s.id, pid, null)` then `onProposalChanged()`; errors surface in the panel's existing inline error. Hidden for co-teach rows and readonly. Edit-history table (Task 6 types) renders `field === 'sling_position_id'` rows as `old_class_name → new_class_name` with a "format" pill.

- [ ] Steps: implement, tsc green, devMock visual check (open day editor, change a format, see end-time/chip update + edit history row), commit `feat: manual format changes in the day editor`

---

### Task 12: Full verification pass

- [ ] `cd src-tauri && cargo test` — all green.
- [ ] `python3 scripts/tests/test_propose_rules.py` — OK.
- [ ] `npx vitest run && npx tsc -b && npm run build` — green.
- [ ] Dev-mock walkthrough (screenshots): settings model select; claude tab full flow (send → checklist → apply all → version card → adopt); algorithm card history; day-editor format change; edits tab shows both edit kinds.
- [ ] Update `docs/data-model.md` cross-references and `CLAUDE.md` (one line under "What this app does" step 3: "…or ask Claude to edit it; recurring patterns get promoted into versioned algorithm rules").
- [ ] Commit `docs: claude editor docs + CLAUDE.md note`, then final review of the diff.

## Self-review notes

- Spec coverage: prerequisites (T3), rules data (T1/T2), version store + archive/delete (T4), generate integration (T5), format edits manual+AI (T6/T11), editor call + validation (T7), code tier + gate (T8), UI (T9/T10/T11), tests/docs (each task + T12). Push-tier unchanged. Out-of-scope items from spec remain out.
- Type names consistent: `ProposedEdit`/`EditorPayload`/`AlgorithmVersion`/`CodeDraft`/`DraftValidation` used identically in Rust (serde) and TS.
- `Rules` weekday validation happens in `validate_rules` (Task 4) AND propose.py trusts its input (already validated before storage; generate reads only stored rules).
```
