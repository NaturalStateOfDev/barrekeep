# Architecture

## Process model

```
┌─────────────────────────────────────────────────────────┐
│  Tauri shell (Rust)                                      │
│  ┌────────────────────────────────────────────────────┐ │
│  │  WebView (Microsoft Edge WebView2 on Windows)       │ │
│  │  ┌──────────────────────────────────────────────┐  │ │
│  │  │  React app (src/)                             │  │ │
│  │  │  - CalendarView                               │  │ │
│  │  │  - ScheduleEditor                             │  │ │
│  │  │  - PromptManager                              │  │ │
│  │  │  - PushPanel                                  │  │ │
│  │  └──────────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────┘ │
│                       │                                   │
│                  Tauri IPC                                │
│                       │                                   │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Rust command handlers (src-tauri/src/)            │ │
│  │  - DuckDB queries (via duckdb crate)                │ │
│  │  - Stronghold (token storage)                       │ │
│  │  - Spawn Python sidecars (Sling pull/push)          │ │
│  │  - Anthropic API calls                              │ │
│  └────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
                        │
              ┌─────────┴────────┐
              ▼                  ▼
       ┌─────────────┐    ┌─────────────────┐
       │  DuckDB     │    │  Python sidecars │
       │  scheduler  │    │  - sling_extract │
       │  .duckdb    │    │  - push_to_sling │
       └─────────────┘    └─────────────────┘
```

## Why this shape

**Why Tauri and not Electron:** Electron ships a 150MB Chromium runtime per app. Tauri uses the OS's WebView2 (already on Windows 10+) and ships ~10MB. Same dev model (HTML/CSS/JS frontend), much smaller install.

**Why React and not Svelte/Solid/vanilla:** The existing widget code is JS/HTML and ports cleanly to React. The library ecosystem for calendar/scheduling components is largest in React. TypeScript adds compile-time safety on the schedule data shape.

**Why DuckDB and not SQLite:** Both are embedded, both work. DuckDB has better performance for the analytical queries this app does (group by teacher, compute weekly load) and natively reads/writes CSV and Parquet, so importing existing CSVs is a one-liner.

**Why Python sidecars and not pure Rust:** The existing Python push/pull scripts work and have been debugged through multiple production runs. Rewriting in Rust is busywork that can be deferred until the scripts grow features that benefit from being in-process. Tauri can shell out to Python with `Command::new`.

## Data flow: a typical month

1. **Pull availability.** User clicks "Pull from Sling for July." Tauri calls the Python sidecar `sling_extract.py`, which fetches calendar events for the target month, parses leave + availability blocks, and writes the result to DuckDB tables `events` and `blocks`.
2. **Generate proposal.** User clicks "Generate proposal." Tauri runs the rule-based proposer (in Rust or Python sidecar) which reads from DuckDB and writes to a `proposals` table with a generation id.
3. **Optional Claude pass.** User clicks "Have Claude review." App reads the proposal, sends it + the prompt from `prompts/verifier.md` to the Anthropic API, and writes Claude's suggestions to a `suggestions` table linked to the proposal.
4. **Edit in calendar view.** User clicks cells, swaps teachers. Each edit becomes a row in the `edits` table (so we have full undo/redo and audit history).
   A month can hold several **drafts** (generate again, or Duplicate a draft for a what-if); the draft menu next to the month title switches, renames, archives and duplicates them, the Compare tab diffs two drafts (changed slots + per-teacher weekday/time consistency), and a Claude prompt can target several drafts at once (one call per draft). See `src-tauri/src/drafts.rs`.
5. **Push to Sling.** User clicks "Push to Sling" on the month's **push draft** (`month_push_candidate`; "Use for push" in the draft menu) — push refuses any other draft, because Sling dedupe would ADD a second draft's differing shifts on top of the first. The app builds the shift list from `proposal_shifts` in DuckDB, dedupes against shifts already in Sling, and POSTs the missing ones in-process (Rust, `sling.rs::push_shift`) as `status: "planning"`, batched + rate-limit-aware. A dry-run preview is shown for confirmation first; live progress streams via the `push-progress` event. Audit goes to the `pushes` and `push_results` tables. (The legacy `scripts/push_to_sling.py` is retained for reference only and is no longer invoked.)
6. **Publish.** User goes to Sling's web UI to publish.

## DuckDB schema overview

See `docs/data-model.md` for full DDL. Tables:

- `teachers` — roster + Sling user IDs + manager overrides
- `positions` — Sling position IDs + class type names + duration
- `availability_blocks` — pulled from Sling per month
- `proposals` — one row per draft (generation run or duplicate), with metadata
- `proposal_drafts` / `month_push_candidate` / `claude_run_targets` — draft names + archive flag, the month's push draft, and which drafts a Claude prompt targeted (migration 0012)
- `proposal_shifts` — the actual generated schedule rows, FK to proposals
- `edits` — every manual edit, with before/after, timestamp, reason
- `prompts` — versioned prompt library (also mirrors prompts/*.md files)
- `claude_runs` — record of every Anthropic API call: prompt, input, output, cost, timestamp
- `pushes` — record of every push-to-Sling run with summary
- `push_results` — per-shift result of each push (Sling shift id, status, error if any)

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
| New UI screen | `src/components/` |
| New shared logic for the frontend | `src/lib/` |
| New TypeScript type | `src/types.ts` |
| New Rust command | `src-tauri/src/commands/` |
| New Python sidecar | `scripts/` |
| New Claude prompt | `prompts/` |
| Architectural decision | `docs/decisions/NNNN-title.md` |
