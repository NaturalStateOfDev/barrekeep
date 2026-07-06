# Claude proposal editor + versioned algorithm (rules & code)

**Date:** 2026-07-06
**Status:** Approved design, pre-implementation

## Goal

Let the lead teacher direct Claude to make concrete edits to a month's
proposal ("give Morgan more Saturdays", "resolve the open conflicts", "make
the Tuesday 5:30 a Classic"), applied through the existing audit-trailed edit
path. On every interaction Claude also self-analyzes: when a request reveals a
*recurring* pattern, it proposes promoting it into the stored algorithm — as a
new versioned rule set (preferred) or, rarely, a new version of the algorithm
script itself. v9 → v10 → v11, each with a human-readable "what changed".

## Change tiers (Claude must prefer the lowest sufficient tier)

1. **Proposal edits** — one-off changes to this month's shifts. No version.
2. **Rules-as-data** — standing rules expressed in the knobs `propose.py`
   already exposes (currently empty): teacher×class blocklist, teacher×slot
   blocklist, hard assignments, priority slots, slot format overrides,
   per-teacher variety multipliers, global variety penalty, weekend time
   shifts. Mints a new version, no code change.
3. **Code change** — new logic the knobs can't express. Claude drafts a full
   new script, which must pass validation before it can be adopted. Mints a
   new version pointing at the new script file.

The prompt requires Claude to justify why a lesser tier is insufficient
before escalating.

## Prerequisites: API key & model selection

- **Anthropic API key required.** Every surface added here (instruction box,
  shortcut, code drafting) gates on the key exactly like today's Review
  button: disabled with "Set your API key in Settings first". The empty
  claude tab teaches the next step instead of erroring.
- **Key moves to Stronghold.** Today the key is memory-only (re-pasted every
  session — the code already notes "Stronghold-backed persistence comes
  later"). This feature makes the key a daily-driver, so it moves to the OS
  keychain using the same mechanism as the Sling token. Settings copy
  updates accordingly; "Clear" removes it from the vault.
- **Model is a user setting, not a constant.** A "Claude model" select on
  the Settings → Anthropic card, stored in a new `app_settings(key, value)`
  table (migration 0010 rides along). Used by review, the proposal editor,
  and code drafting alike; replaces the hardcoded `REVIEW_MODEL`
  (`claude-sonnet-4-6`) in review.rs. Curated options with rough per-call
  cost shown in the description (typical call ≈ 15k in / 2k out):
  - `claude-opus-4-8` — **default**; most capable (~13¢/call). Edits must
    reference real shift ids and drafted code runs against real data, so
    correctness is worth it at this volume (a handful of calls per month).
  - `claude-sonnet-4-6` — balanced (~7¢/call); today's review model.
  - `claude-haiku-4-5` — cheapest (~2–3¢/call).
  Unknown/stale stored values fall back to the default at call time.

## UX (frontend)

The Proposals "review" tab becomes the **claude** tab:

- **Instruction box** + Send, plus a **"Resolve open conflicts"** shortcut
  that pre-fills the instruction. "Have Claude review" (existing advisory
  review) stays as-is below.
- **Result panel** after a call:
  - *Edits checklist*: each proposed edit shows the slot, the action
    (reassign / unassign / change format), and a one-line rationale, with
    per-edit **Apply** and a top-level **Apply all**. Applied edits go
    through the existing edit commands with reason `claude: <rationale>` —
    fully visible in Edit history and individually revertable.
  - *Version proposal card* (only when present): description, rule diff
    (and code diff + validation summary for code changes), **Adopt as vN**
    button. Rejecting costs nothing; the raw output is already logged in
    `claude_runs`.
  - *needs_code_change card*: rationale + **"Draft code change"** button
    that triggers the second, code-bearing call.
- **Algorithm card** (same tab): active version + description, expandable
  rule list, version history with created-by / adopted date / last-used
  month (derived from `proposals`), per-version **Delete** (non-active
  only), and archive status.
- **Day editor**: a **"Change format"** action beside "Change teacher" —
  picker of schedulable class types (chips); manual counterpart of Claude's
  `change_format` edits.

## Data model (migration 0010)

Append-only, no FKs, no UNIQUE beyond the PK, rows never UPDATEd
(per the DuckDB rules in CLAUDE.md / the schema-change skill):

```sql
CREATE TABLE app_settings (
  key        VARCHAR PRIMARY KEY,   -- e.g. 'claude_model'
  value      VARCHAR NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE algorithm_versions (
  version       INTEGER PRIMARY KEY,       -- 10, 11, ... (v9 = shipped baseline)
  description   VARCHAR NOT NULL,          -- "v10 — Casey off Reform; Sat opener 8:15"
  rules         JSON NOT NULL,             -- FULL ruleset snapshot (not a delta)
  script_file   VARCHAR,                   -- NULL = shipped scripts/propose.py;
                                           -- else file name under <app_data>/algorithms/
  created_by    VARCHAR NOT NULL,          -- 'claude' | 'user'
  claude_run_id BIGINT,                    -- provenance into claude_runs (app-enforced)
  adopted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- Active version = `max(version)`. Rows are inserted only on explicit
  adoption; un-adopted proposals live only in the UI + `claude_runs` log
  ("light history").
- "Last used" per version = `max(p.generated_at)` over proposals whose
  `algorithm_version = 'vN'` — derived, never stored.
- `proposals.algorithm_version` stores the label (`"v10"`); its existing
  `parameters` JSON records the exact rules used for that generation.

### Rules JSON schema (v1)

```json
{
  "teacher_class_blocklist":  [{"sling_user_id": 0, "class_name": "", "reason": ""}],
  "teacher_slot_blocklist":   [{"sling_user_id": 0, "weekday": "Sat", "time": "08:00", "reason": ""}],
  "hard_assignments":         [{"weekday": "", "time": "", "class_name": "", "sling_user_id": 0, "reason": ""}],
  "priority_slots":           [{"weekday": "", "time": "", "sling_user_id": 0}],
  "slot_class_overrides":     [{"weekday": "", "time": "", "class_name": ""}],
  "variety_penalty_multiplier": {"<sling_user_id>": 1.0},
  "variety_penalty_per_class": 0.3,
  "sat_time_shifts": {}, "sun_time_shifts": {}
}
```

All keys optional; unknown keys are rejected by the Rust-side validator
before adoption (guards against prompt drift). Exact knob shapes are matched
to propose.py's consuming code during implementation.

## Algorithm script handling

- **Baseline**: the shipped `scripts/propose.py` stays untouched — permanent
  known-good fallback (label `v9`).
- **Versioned scripts**: `<app_local_data>/algorithms/propose_vN.py`, with
  `algorithms/archive/` beside it. Resolution order at generate time:
  `algorithms/` → `algorithms/archive/` → error ("script deleted — adopt a
  newer version").
- **Payload extension**: the stdin payload gains optional `"rules": {...}`
  and `"version_label": "vN"`. propose.py populates its override knobs from
  `rules` and echoes `version_label` as `algorithm_version`.
  **Regression invariant: empty/absent rules ⇒ byte-identical output to
  today's v9** (tested).
- **Validation gate for code versions** (before Adopt is enabled):
  1. run the candidate script on the most recent generated month's payload
     (rebuilt from the DB by the same builder `generate_proposal` uses);
     it must exit 0 and emit schema-valid JSON;
  2. show a diff summary vs. that month's actual proposal (N assignments
     changed, N slots added/removed) plus the unified code diff vs. the
     current script;
  3. adoption writes the file + inserts the version row.
- **Archive sweep** at startup: script files > 3 versions behind active AND
  last used > 3 months ago (or never) move to `algorithms/archive/`.
  Deletion is manual-only, from the Algorithm card, never the active
  version. Deleting a script never touches proposal history.

## Claude calls (Rust, `review.rs` pattern: blocking ureq, logged to `claude_runs`)

**Call 1 — `claude_edit_proposal(proposal_id, instruction)`**
Payload: schedule rows (with shift ids), roster + caps, qualifications,
availability blocks, current issues, edit history, active rules, instruction.
Structured response:

```json
{
  "summary": "…",
  "edits": [{
    "proposal_shift_id": 0,
    "action": "reassign" | "unassign" | "change_format",
    "new_user_id": 0,          // reassign
    "new_class_name": "",      // change_format (mapped to position id app-side)
    "rationale": ""
  }],
  "ruleset_proposal": null | {"description": "", "rules": { /* full new set */ }},
  "needs_code_change": null | {"rationale": ""}
}
```

App-side validation before showing: shift ids must exist in the proposal,
teachers must exist (warn-not-block on qualification mismatches — Claude may
be intentionally overriding), class names must map to schedulable positions.

**Call 2 — `claude_draft_code_change(proposal_id, instruction, rationale)`**
Adds the current script source to the context. Response: full new script +
description. Kept separate so routine edit calls never pay the ~8k-token
script cost.

**Prompt**: `prompts/proposal-editor.md`, read from disk at runtime (project
root), falling back to an inline copy in Rust if missing — first step toward
the prompts-library wiring CLAUDE.md describes; lets the user tune wording
without recompiling.

## New/changed Rust commands

- `edit_proposal_shift_position(proposal_shift_id, new_position_id, reason)`
  — sets `sling_position_id`, recomputes `end_time` from the new position's
  `duration_minutes`, records an `edits` row with `field='sling_position_id'`
  (values = position ids; UI renders class names). Blocked for co-teach rows
  (same as teacher edits). Safe post-0009: no indexes touched.
- `claude_edit_proposal`, `claude_draft_code_change` (above).
- `list_algorithm_versions`, `adopt_algorithm_version(description, rules,
  script_content?, claude_run_id?)` (validates rules schema; assigns
  version = max(existing, 9) + 1; writes the script file when present;
  inserts the row), `delete_algorithm_script(version)`.
- `get_app_setting(key)` / `set_app_setting(key, value)` (INSERT OR REPLACE;
  no FKs/extra indexes on the table, so safe under the DuckDB rules).
  Claude calls read `claude_model` at call time, defaulting to
  `claude-opus-4-8`; review.rs's `REVIEW_MODEL` constant becomes the
  fallback path only.
- `set_anthropic_key` / `has_anthropic_key` gain Stronghold persistence
  (mirror the Sling-token commands); the in-memory Mutex stays as the
  session cache, preloaded from the vault at startup like the Sling token.
- `generate_proposal`: resolve active version → script path + rules →
  include in payload → spawn resolved script.
- Frontend applies edit checklists by looping the two single-edit commands
  (each ~ms locally; keeps one audit path).

## Edit history / UI plumbing

- `EditRow` gains class-name rendering for `sling_position_id` edits
  (From/To shown as class names via join on positions).
- `types.ts` + `api.ts` additions mirroring the new commands.

## Testing

- Rust: rules-schema validator (accept/reject), version resolution
  (baseline vs file vs archived vs missing), `edit_proposal_shift_position`
  end-time recompute + audit row + co-teach block, archive-sweep selection
  logic, response-validation of Claude edit JSON (fixture).
- Python: `propose.py` with empty rules ⇒ identical JSON to no-rules run
  (fixture-based); each rule knob exercised once.
- Frontend: checklist apply-all flow against the dev mock; format picker in
  the day editor.

## Out of scope (this iteration)

- Editing shift times/adding/removing slots via Claude (edits are
  teacher/format/drop only; slot-shape changes belong to rules or code tiers).
- Automatic adoption of anything. Every version and every edit batch is
  user-approved (with fast Apply-all / Adopt buttons).
- Syncing prompts/ into the DB (`prompt_id` stays NULL in `claude_runs`).
- Cloud backup of the algorithms dir (rides along with the future DB backup).
