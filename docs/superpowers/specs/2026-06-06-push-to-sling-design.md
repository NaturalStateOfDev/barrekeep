# Push to Sling — design

**Date:** 2026-06-06
**Status:** approved (design); implementation plan to follow
**Touches:** Sling integration (read `docs/sling-api.md` + the `sling-integration` skill before coding)

## Problem

Barrekeep's stated workflow is **Pull → Propose → Review → Push**. The first
three are wired end to end. **Push is not**: `pushes`/`push_results` tables
exist (migration 0001) and `scripts/push_to_sling.py` exists, but there is no
Tauri command, no `api.ts` entry, and no UI. The script is also stale — it
reads a `--csv` file with hardcoded `ORG_ID`/`ACTING_USER`/`ROSTER`/`POSITIONS`
and hardcoded June-2026 `viewdates`, predating the runtime-config externalization
(`studio_config`) and the stdin-JSON pattern the rest of the app uses.

The app cannot complete its core job until Push works.

## Decision

Implement Push as a **pure-Rust port** in `src-tauri/src/sling.rs`, reusing the
existing HTTP client, browser headers, `CalendarEvent` DTO, flex
string-or-int deserializers, and `StudioConfig`. The Python `push_to_sling.py`
and `rollback_push.py` scripts are retired from the runtime path (kept in
`scripts/` for reference only; not invoked).

Rationale: the user chose the in-process port for native progress streaming.
`docs/architecture.md:47` argues for keeping debugged Python, so the mitigation
is **faithful replication** of every request shape, verified by unit tests
against captured fixtures plus an end-to-end manual run against a real Sling
session.

### Approaches considered

1. Python sidecar, single JSON result (mirror `propose.py`) — smallest change,
   but no live progress; rejected because the chosen UX needs per-batch progress.
2. Python sidecar + streamed JSONL progress — keeps debugged Python, adds
   streaming; viable but more sidecar plumbing.
3. **Pure-Rust port (chosen)** — in-process, native progress, drops the Python
   dependency for push; risk is re-implementing debugged logic, mitigated by
   exact replication (unit-tested against fixtures) + an end-to-end manual run.

## API fidelity (must match `push_to_sling.py` / `docs/sling-api.md` exactly)

| Detail | Value |
|---|---|
| POST | `POST /v1/{org}/shifts?user-fields=id&checkRestBreakConflicts=true&viewdates=…&cachedates=…&checkConsecutiveWorkDaysConflicts=true` |
| Body | `{location:{id}, dtstart, dtend, users:[{id}], slots:1, position:{id}, status:"planning"}` — `users` is an **array** on POST |
| dtstart/dtend | naive local `YYYY-MM-DDTHH:MM`, **no offset** (Sling applies tz on echo); built from `proposal_shifts.shift_date` + `start_time`/`end_time` |
| Response | array → unwrap `[0]` → read `.id` (flex string-or-int) |
| Dedupe GET | reuse existing calendar GET; filter `type=="shift"`, `location.id == home_location_id` (exact match, skip no-location), `status ∈ {planning, published}`; fingerprint `(date, HH:MM, user_id, position_id, location_id)` |
| DELETE (documented for a future rollback feature; not built this iteration) | `DELETE /v1/{org}/shifts/{id}?viewdates=…&cachedates=…` → 204 |
| Headers | same browser/Cloudflare set already in `sling.rs` (UA, Origin, Referer, Sec-Fetch-*) |
| Rate limit | batch 10, 1s intra / 10s inter, 429 backoff `30·attempt`, **max 3 retries**, then log-and-skip |

**Deliberate change from the stale script:** `viewdates`/`cachedates` are
computed from the proposal's month instead of hardcoded June 2026 —
`viewdates_start = firstOfMonth − 1 day`, `viewdates_end = firstOfNextMonth + ~4 days`,
`cachedates` one day wider on each side. **Preserve the exact string formats the
web client sends:** `viewdates`/`cachedates` use `-0500` (no colon); the calendar
`dates=` param uses `-05:00` (with colon). Org / acting-user / home-location come
from `studio_config`, never constants. DST remains the existing fixed-offset TODO.

## Architecture / data flow

```
ProposalView ──"Push to Sling"──▶ push_proposal_dry_run(proposalId)   [Rust, sync]
                                   ├─ build specs from proposal_shifts (DuckDB)
                                   ├─ gate: error if any non-dropped shift unassigned
                                   ├─ GET calendar, dedupe → {to_create, skipped}
                                   └─ returns PushPreview (zero writes)
   user reviews preview ──"Confirm"──▶ push_proposal_execute(proposalId)  [Rust, bg thread]
                                   ├─ INSERT pushes row (started_at)
                                   ├─ for each batch/shift: POST → emit "push-progress",
                                   │     INSERT push_results row (outcome, sling_shift_id|error)
                                   ├─ on 401: stop, emit "sling-401" → SlingTokenModal
                                   └─ UPDATE pushes counts + finished_at; emit "push-done"
```

### Components

- **`sling.rs`** (new): `push_shift(token, cfg, spec) -> Result<i64>` (returns
  Sling shift id), `push_month` driver (batching/backoff/dedupe), `PushSpec`,
  `PushPreview`, `PushProgress` types, and `view_cache_dates(month)` helper.
  Reuses `http_get_with_query`; adds `http_post` (ureq `send_json`).
- **`commands.rs`** (new commands): `push_proposal_dry_run` and
  `push_proposal_execute`. Both load `studio_config`, build specs from
  `proposal_shifts` joined to `teachers`/`positions`. Execute runs on a
  background thread (same deferred-thread + `emit` pattern as the
  `sling_login.rs` fix), streaming `push-progress` events. No partial/limit
  mode — a full month is always pushed (idempotent dedupe makes that safe).
- **`api.ts`**: `pushProposalDryRun(proposalId)`, `pushProposalExecute(proposalId)`.
- **`PushModal.tsx`**: preview list (to-create rows: date/time, class, teacher,
  plus skipped count) → Confirm → live progress bar + current-shift line →
  summary. Listens to `push-progress`; routes 401 to the existing
  `SlingTokenModal`.
- **Proposal toolbar**: a "Push to Sling" button that opens `PushModal`.

### Idempotent retry (partial failure)

Execute always re-runs dedupe first, so re-pushing after a partial failure only
POSTs the fingerprints still missing from Sling. No explicit resume/rollback
state machine. The summary reports created / failed / skipped.

## Safety checks

1. **`status:"planning"` is a hard-coded literal** — not a parameter; no code
   path can publish. (Project rule: manager publishes from Sling's UI.)
2. **Dry-run-first is structural** — `execute` is a separate command; the UI
   cannot reach it without first rendering the preview; `dry_run` issues zero
   writes.
3. **Unassigned gating** — dry-run hard-errors if any non-dropped shift lacks a
   teacher (`sling_user_id IS NULL`); dropped shifts (`is_dropped`) skipped.
   Flagged-but-assigned shifts are allowed (flag is advisory).
4. **Dedupe before every POST** — no idempotency key exists; never POST a
   fingerprint already present at the home location.
5. **Rate-limit ceiling respected** — never more than 3 retries per shift;
   batch/backoff values unchanged from the proven script.
6. **Full audit** — `pushes` + `push_results` rows written incrementally so a
   crash mid-run still leaves a record.
7. **401 mid-run** — stop gracefully, show what already landed, trigger the
   existing `SlingTokenModal` re-login flow; user re-pushes (idempotent).

## Testing

- **Rust unit tests:** `fingerprint` dedupe; spec-building from a proposal
  fixture; the unassigned-gate error; `view_cache_dates` computation + exact
  string format (`-0500` vs `-05:00`); POST-body shape (assert `users` is an
  array and `status == "planning"`) against a captured fixture.
- **Manual end-to-end run:** against a real Sling session, run a full-month
  dry-run, review the preview, then execute and confirm the planning shifts
  appear in Sling's UI. This is the validation round before relying on the
  feature for a real monthly schedule.

## Out of scope

- Publishing shifts (manager does this in Sling's UI).
- A persistent push-history view (only "last pushed …" surfaced for now).
- Rollback — neither a `delete_shift` client nor a rollback UI is built this
  iteration. The DELETE shape is documented in the API table above for when a
  rollback feature is taken up.
- DST-correct offsets (existing fixed `-05:00`/`-0500` TODO carries forward).
