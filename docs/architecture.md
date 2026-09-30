# Architecture

## Process model

```
┌─────────────────────────────────────────────────────────┐
│  Tauri shell (Rust)                                      │
│  ┌────────────────────────────────────────────────────┐ │
│  │  WebView (Microsoft Edge WebView2 on Windows)       │ │
│  │  ┌──────────────────────────────────────────────┐  │ │
│  │  │  React app (src/)                             │  │ │
│  │  │  - screens/ (month calendar, settings, ...)   │  │ │
│  │  │  - components/ (calendar, claude tab,         │  │ │
│  │  │    compare view, push modal, studio setup)    │  │ │
│  │  └──────────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────┘ │
│                       │                                   │
│                  Tauri IPC                                │
│                       │                                   │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Rust command handlers (src-tauri/src/)            │ │
│  │  - DuckDB queries (duckdb crate, bundled engine)    │ │
│  │  - Stronghold (token storage)                       │ │
│  │  - Sling pull / push / sync (in-process HTTP)       │ │
│  │  - Anthropic API calls                              │ │
│  │  - Spawn propose.py (python.rs finds Python)        │ │
│  └────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
          │                  │                    │
          ▼                  ▼                    ▼
   ┌─────────────┐   ┌───────────────┐   ┌─────────────────┐
   │  DuckDB     │   │  Sling API    │   │  propose.py      │
   │  scheduler  │   │  (HTTPS via   │   │  (subprocess,    │
   │  .duckdb    │   │   ureq)       │   │   JSON on stdin) │
   └─────────────┘   └───────────────┘   └─────────────────┘
```

Rust modules at a glance (`src-tauri/src/`):

| Module | Role |
|---|---|
| `commands.rs` | Most Tauri commands: pull, generate, edits, Claude, settings |
| `sling.rs` | Sling HTTP: browser-like headers, roster/calendar pull, shift POST/DELETE, dedupe, rate limiting, studio-TZ offsets |
| `push_sync.rs` | Incremental push ("sync") of the push draft, cleanup, "remove from Sling" |
| `drafts.rs` | Multiple drafts per month; the push draft (`month_push_candidate`) |
| `conflicts.rs` | Re-check a draft against freshly pulled availability |
| `studio_setup.rs` | Auto-detect org / acting user / home location after a Sling login |
| `algorithm.rs` | Versioned algorithm rules and scripts (`algorithm_versions`) |
| `review.rs`, `editor.rs` | Claude review; Claude-driven draft edits |
| `python.rs` | Find a usable Python 3.11+ for `propose.py` |
| `backup.rs` | Rotating database backups |
| `migrations.rs`, `db.rs` | Forward-only schema migrations; connection setup |
| `secrets.rs`, `sling_login.rs` | Stronghold vault; in-app Sling login window |

## Why this shape

**Why Tauri and not Electron:** Electron ships a 150MB Chromium runtime per app. Tauri uses the OS's WebView2 (already on Windows 10+) and ships ~10MB. Same dev model (HTML/CSS/JS frontend), much smaller install.

**Why React and not Svelte/Solid/vanilla:** The existing widget code is JS/HTML and ports cleanly to React. The library ecosystem for calendar/scheduling components is largest in React. TypeScript adds compile-time safety on the schedule data shape.

**Why DuckDB and not SQLite:** Both are embedded, both work. DuckDB has better performance for the analytical queries this app does (group by teacher, compute weekly load) and natively reads/writes CSV and Parquet, so importing existing CSVs is a one-liner. The engine is compiled into the app together with its `json` extension, and extension autoinstall/autoload is off (`db.rs`), so the app never downloads DuckDB extensions at runtime. ICU is not bundled — stick to core SQL functions for TIMESTAMPTZ (e.g. `epoch_us`, not `epoch`).

**Why Sling in Rust but the algorithm in Python:** Sling pull/push began as Python scripts and moved in-process (`sling.rs`, `push_sync.rs`) so the app can stream progress, read the token from Stronghold and write DuckDB directly; the old scripts are kept for reference in `scripts/legacy/` and are never invoked. The schedule algorithm (`scripts/propose.py`) stays in Python: it changes most often, is easy to test on its own, and Claude can draft changes to it (`algorithm_versions`). The app runs it as a subprocess with a JSON payload, built from DuckDB, on stdin.

## Data flow: a typical month

1. **Pull availability.** User clicks "Pull from Sling." The app (`commands.rs::pull_month_from_sling` over `sling.rs`) fetches the roster, position groups, the target month's calendar events and 3 months of shift history, keeps only the home location, and writes `teachers` / `positions` / `teacher_qualifications` (roster sync), `availability_blocks` (Sling `availability` = BLOCKED time), `external_sling_shifts` and `month_pulls`. After a Sling login the app also auto-detects the studio configuration (`studio_setup.rs`); it never overwrites a configuration that is already set.
2. **Generate proposal.** User clicks "Generate." The app builds a JSON payload from DuckDB, runs `propose.py` (found via `python.rs`) with it on stdin, and stores the result as a new draft: a `proposals` row plus its `proposal_shifts`.
3. **Optional Claude pass.** "Have Claude review" sends the draft + `prompts/verifier.md` to the Anthropic API; the Claude tab can also edit the draft from an instruction, or propose rule / code changes to the algorithm. Every call is logged in `claude_runs` (cost audit).
4. **Edit in calendar view.** User clicks cells, swaps teachers. Each edit becomes a row in the `edits` table (so we have full undo/redo and audit history).
   A month can hold several **drafts** (generate again, or Duplicate a draft for a what-if); the draft menu next to the month title switches, renames, archives and duplicates them, the Compare tab diffs two drafts (changed slots + per-teacher weekday/time consistency), and a Claude prompt can target several drafts at once (one call per draft). See `src-tauri/src/drafts.rs`.
   **Refresh availability** re-pulls Sling and re-checks an existing draft against it (`conflicts.rs`, `draft_checks`) instead of regenerating; a draft whose month was pulled again since it was generated or last checked shows as stale.
5. **Push to Sling.** User clicks "Push to Sling" on the month's **push draft** (`month_push_candidate`; "Use for push" in the draft menu) — push refuses any other draft, because Sling dedupe would ADD a second draft's differing shifts on top of the first. Push is an incremental **sync** (`push_sync.rs`): it creates the draft's missing shifts and replaces (DELETE + POST) or removes shifts the app created earlier whose slot changed — always as `status: "planning"`, batched + rate-limit-aware. It never touches a shift that is published, missing from Sling, not created by the app, or edited in Sling since the last push (compared against `push_result_snapshots`). A dry-run preview is shown for confirmation first; live progress streams via the `push-progress` event. A backup is taken before any change; audit goes to `pushes`, `push_results` and `push_result_snapshots`.
6. **Publish.** User goes to Sling's web UI to publish.

## DuckDB schema overview

See `docs/data-model.md` for full DDL. Tables:

- `teachers` — roster + Sling user IDs + manager overrides
- `teacher_qualifications` — who can teach which position (from Sling position groups)
- `positions` — Sling position IDs + class type names + duration
- `studio_config` — Sling org / acting-user / home-location ids (runtime config)
- `availability_blocks` — pulled from Sling per month
- `external_sling_shifts` / `month_pulls` — shifts already in Sling; when each month was last pulled
- `proposals` — one row per draft (generation run or duplicate), with metadata
- `proposal_drafts` / `month_push_candidate` / `claude_run_targets` — draft names + archive flag, the month's push draft, and which drafts a Claude prompt targeted (migration 0012)
- `proposal_shifts` — the actual generated schedule rows, FK to proposals
- `edits` — every manual edit, with before/after, timestamp, reason
- `draft_checks` — when each draft was last re-checked against pulled data (migration 0013)
- `prompts` — versioned prompt library (also mirrors prompts/*.md files)
- `claude_runs` — record of every Anthropic API call: prompt, input, output, cost, timestamp
- `algorithm_versions` / `app_settings` — versioned algorithm rules and scripts; misc settings
- `pushes` — record of every push-to-Sling run with summary
- `push_results` — per-shift result of each push (Sling shift id, status, error if any)
- `push_result_snapshots` — what each app-created shift looked like when pushed, so sync can tell whether it was edited in Sling (migration 0013)

## State that lives outside DuckDB

- **Secrets:** Sling token, Anthropic API key — Stronghold (OS keychain).
- **User preferences:** window size, last-viewed month — Tauri config dir as JSON.
- **Prompt source files:** `prompts/*.md` — git-versioned, copied into DuckDB on app startup if newer.

## Python runtime

`propose.py` needs Python 3.11+ (stdlib only). `src-tauri/src/python.rs`
probes, in order, `py -3`, `python`, `python3` on Windows (`python3`, `python`
elsewhere), rejects the Microsoft Store placeholder (`WindowsApps\python.exe`,
exit 9009) and anything older than 3.11, and caches the first good one for the
session. Settings → Python shows the result and re-probes on "Re-check".
Because Windows apps inherit PATH at launch, installing Python requires an
app restart before it's picked up.

## Backups

The database lives at `%LOCALAPPDATA%\com.barrekeep.app\scheduler.duckdb`.
Two kinds of copies sit next to it:

- `backups/scheduler-YYYYMMDD-HHMMSS-<reason>.duckdb` — routine backups
  (`src-tauri/src/backup.rs`). Taken once per calendar day at startup
  (`startup`), before every Sling sync that changes shifts (`prepush`; `preremove` for
  "remove this draft from Sling"), and
  from Settings → Backups → "Back up now" (`manual`). The newest 14 are kept;
  rotation deletes only files matching that name pattern. They are written
  through the open connection with DuckDB's
  `ATTACH '<file>' AS b; COPY FROM DATABASE scheduler TO b; DETACH b` — never
  by copying the live file, which Windows refuses while it's open (os error
  32). A failed backup is logged to `logs\barrekeep.log` and shown as a
  warning; it never blocks startup or a push.
- `scheduler.duckdb.backup-vN` — taken automatically right before a schema
  migration (`migrations::backup_if_pending`), N = the schema version it holds.

### Restoring a backup

There is no in-app restore; do it by hand:

1. **Quit Barrekeep completely** (check Task Manager) — the database
   file can't be replaced while it's open.
2. In `%LOCALAPPDATA%\com.barrekeep.app\`, rename the current
   `scheduler.duckdb` to e.g. `scheduler.duckdb.before-restore` (keep it until
   you're sure). If a `scheduler.duckdb.wal` file exists, rename it alongside.
3. Copy the chosen file from `backups\` into that folder and rename the copy
   to `scheduler.duckdb`.
4. Start Barrekeep. If the backup predates a schema change, pending migrations
   run automatically on startup (making their own `.backup-vN` first).

A backup is a normal DuckDB file of the same engine version, so it can also be
inspected read-only with the DuckDB CLI of the pinned version (see
`src-tauri/Cargo.toml`) without restoring it.

## Where to put new code

| What | Where |
|---|---|
| New UI screen | `src/screens/` (reusable pieces in `src/components/`) |
| New shared logic for the frontend | `src/lib/` |
| New TypeScript type | `src/types.ts` |
| New Rust command | `src-tauri/src/commands.rs`, or the feature's module (`drafts.rs`, `push_sync.rs`, ...), registered in `lib.rs` |
| New Sling API call | `src-tauri/src/sling.rs` (read `docs/sling-api.md` first) |
| Schema change | new `src-tauri/migrations/NNNN_*.sql` + `migrations.rs` |
| Algorithm change | `scripts/propose.py` (+ `scripts/tests/test_propose_rules.py`) |
| New Claude prompt | `prompts/` |
| Architectural decision | `docs/decisions/NNNN-title.md` |
