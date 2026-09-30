# Barrekeep — barre studio scheduler

Barrekeep is a desktop scheduling tool for a single barre studio. The studio's
lead teacher uses it once a month to build the class schedule and push it to
Sling as planning-status (unpublished) shifts.

## What this app does

1. **Pull** teacher availability from Sling for the upcoming month
2. **Propose** a draft schedule using rule-based generation (with Claude as a tunable second opinion via the prompts library)
3. **Review** the draft in a calendar UI; edit teacher assignments, swap
   classes and formats, flag conflicts — or ask Claude to edit it (claude tab).
   Recurring patterns get promoted into versioned algorithm rules
   (`algorithm_versions`, v9 = baseline; see `src-tauri/src/algorithm.rs`)
4. **Push** the approved draft to Sling as planning-status shifts (manager publishes from Sling's UI later)

The app is single-user, local-first, and runs on the user's Windows laptop. No
server, no cloud database.

## Stack

- **Shell:** Tauri (Rust-based, ships a small native installer)
- **Frontend:** React + TypeScript + Vite, plain CSS (no Tailwind)
- **Storage:** DuckDB embedded — one file, `scheduler.duckdb`, in the app's local
  data dir (`%LOCALAPPDATA%\com.barrekeep.app\` on Windows,
  `~/.local/share/com.barrekeep.app/` on Linux; see `db.rs`)
- **Secrets:** Tauri Stronghold plugin (OS keychain, never on disk in plaintext)
- **AI:** Anthropic SDK for prompt-driven schedule analysis
- **Sling integration:** in-process Rust (`src-tauri/src/sling.rs` — pull, push,
  dedupe, rate limiting; `push_sync.rs` — incremental push/sync). Only
  `scripts/propose.py` (the schedule algorithm) runs as a subprocess, fed a
  JSON payload over stdin; `python.rs` finds a Python 3.11+ to run it.

## Repository layout

See `docs/architecture.md` for the full map. The summary:

- `src/` — React frontend
- `src-tauri/` — Rust shell + Tauri config. Beyond `commands.rs`, `sling.rs`
  and `migrations.rs`: `drafts.rs` (multi-draft + push draft), `push_sync.rs`
  (Sling sync), `conflicts.rs` (re-check a draft against fresh availability),
  `backup.rs` (rotating DB backups), `python.rs` (Python discovery),
  `studio_setup.rs` (studio-config auto-detect)
- `scripts/` — `propose.py` is the schedule algorithm, invoked with a JSON
  payload over stdin; `tests/` holds its regression + release smoke tests.
  `legacy/` keeps the retired Python Sling scripts for reference only
  (superseded by `sling.rs` + `push_sync.rs`; never run them).
- `prompts/` — Markdown files, one per Claude prompt (proposer, verifier).
  Versioned in git; read at runtime.
- `data/` — all local-only (gitignored): local pulls/fixtures for dev. The
  app's database is NOT here (see Storage above). Nothing under `data/` ships.
- `docs/` — architecture, Sling API notes, data model
- `.claude/` — skills and subagents used by Claude Code when working on this repo

## Key constraints

- **Studio identifiers are runtime config, not compiled in.** The Sling org id,
  acting-user id, and home-location id live in the `studio_config` table
  (Settings → Studio configuration; auto-detected after a Sling login, never
  overwritten once set). A pull errors until they're set.
- **Sling rate limits.** Aggressive. Push must be batched (10 shifts per batch,
  10s pause between batches, 30/60/90s backoff on 429). See `sling.rs` and
  `push_sync.rs`.
- **Sling auth tokens expire** and are refreshed via the in-app Sling login (or
  pasted from a browser DevTools session). There is no programmatic OAuth flow.
- **No publishing.** The app creates shifts as `status: "planning"` only. A
  manager publishes from Sling's UI after final review.
- **Single home location.** Only the configured home location is kept; other
  locations in the same Sling org are filtered out.
- **Teacher qualifications** come from Sling's position groups, not teaching
  history. Treat Sling positions as ground truth for "who can teach what."
- **Co-teaching** is two separate shift records at the same time slot in Sling.
  There is no co-teach flag in Sling's data model.

## Reference data

The class-type/position mapping, weekly cap defaults, and special scheduling
rules are documented in `docs/data-model.md`. There is no seed data — a fresh
install starts empty on purpose (`src-tauri/src/seed.rs` is intentionally a
no-op) so it's obvious whether Sling is actually connected. The roster,
positions, and qualifications all arrive via the Sling pull or roster refresh.

## Working on this project

When you (Claude) edit code in this repo:

1. **Read `docs/architecture.md` and `docs/sling-api.md` first** if your change
   touches the data model or Sling integration.
2. **Don't introduce new top-level dependencies casually.** This is a small,
   personal app. Justify additions in commit messages.
3. **Prefer plain CSS** over Tailwind or styled-components. The widget code uses
   Anthropic's design-token CSS variables; that pattern continues.
4. **Match the existing Python scripts' style** in `scripts/`: type hints,
   urllib over requests (no extra dependency), explicit error handling, JSON
   audit logs.
5. **Never delete from `scheduler.duckdb` without a backup.** The schedule
   history is months of work. `backup.rs` keeps 14 rotating backups
   (`backups/` next to the DB: daily at startup, before every Sling push, and
   Settings → Back up now); take one before any manual surgery.

## Known gotchas (the kind that bite at 11pm)

- **The `availability` event type in Sling means BLOCKED time, not available
  time.** The naming is backward.
- **Sling's POST `/shifts` uses `users: [{id}]` (array) but PUT uses
  `user: {id}` (singular).** Not symmetric. The response shape uses singular.
- **Sling's API responses are always arrays**, even for single-shift creates.
  Unwrap `resp[0]`.
- **Sling stringifies large numeric IDs** (e.g. event ids) to preserve JS
  precision — parsers must accept either a string or a number.
- **Cloudflare blocks default User-Agents.** HTTP requests must send
  browser-like headers (User-Agent, Origin, Referer, Sec-Fetch-*).
- **DST transitions.** The studio observes US Central Time (America/Chicago).
  Never hard-code `-05:00`/`-0500` — it's only right during CDT. Rust derives
  every offset from `sling::STUDIO_TZ` (chrono-tz) via `studio_iso` /
  `month_range` / `view_cache_dates`, each boundary with its OWN offset
  (November 2026 = `…11-01T00:00:00-05:00` to `…11-30T23:59:59-06:00`).
  `propose.py` uses a stdlib `_USCentral` tzinfo (see the tzdata gotcha
  below). Push bodies stay naive local times — Sling applies the
  zone. The legacy `scripts/legacy/*.py` scripts still carry fixed
  June-2026 `-05:00` constants.
- **DuckDB UPDATEs are landmines near indexes and foreign keys.** An UPDATE
  that touches an indexed column (UNIQUE/PK) is executed as DELETE+INSERT
  internally, and fails with "still referenced by a foreign key in a
  different table" if any row references it. Migrations 0003, 0004, and 0009
  each removed constraints for this reason. Rules: never put UNIQUE (beyond
  the PK) or incoming FKs on a table whose rows get UPDATEd; never UPDATE a
  PK; prefer compare-before-write so unchanged rows are not touched at all.
  The engine version is tilde-pinned in `src-tauri/Cargo.toml` (crate
  `1.1MMPP` = libduckdb `1.MM.PP`) because engine bumps change the on-disk
  format and are not backward-readable — bump deliberately, via the
  dependabot PR. The `json` extension is compiled in; ICU is NOT (not
  bundleable from the crates.io crate) and extension autoload is off
  (`db.rs`), so ICU-only SQL like `epoch(TIMESTAMPTZ)`, `date_trunc` on a
  TIMESTAMPTZ, or TIMESTAMPTZ ± INTERVAL fails — use core functions
  (`epoch_us`) or do the date math in Rust. Read TIMESTAMPTZ as text only
  via `db::utc_iso!(col)` (ISO UTC `…Z`; the frontend localizes it), and
  compute "this month" with `sling::studio_month_at`, never SQL `now()`
  (UTC — wrong after 7pm Central on a month's last day).
- **Windows file locks are mandatory; never touch `scheduler.duckdb` on disk
  while a connection is open.** Copying, moving, or deleting the database
  file (or its WAL) while any DuckDB connection holds it fails on Windows
  with "being used by another process (os error 32)". Linux locks are
  advisory, so the identical code passes on the dev machine and in any
  Linux test run — "works on Linux" is no evidence for file-handling code.
  This was the v0.2.0–0.2.2 instant-crash-at-startup: the pre-migration
  backup copied the file while the app's own connection was open, on every
  launch, and the schema never advanced. Pattern: open a short-lived
  connection, `CHECKPOINT`, explicitly `close()` it, then do the file op
  (see `migrations::backup_if_pending`). The Rust tests only reproduce
  this on a Windows runner.
- **Tauri 2 runs plain synchronous `#[tauri::command]`s on the main (UI)
  thread.** Anything slow — HTTP (Sling, Anthropic), spawning Python, DB
  copies/backups — must be `#[tauri::command(async)]` or an `async fn`, or
  the whole window freezes until it returns.
- **Stock Windows Python has no tzdata**, so `propose.py` can't use
  `zoneinfo`; it carries a built-in US Central table (`_USCentral`). Keep it
  stdlib-only — the studio PC has a bare python.org install.
- **Sling sync only touches shifts it owns and nobody has touched.** Push
  never updates or deletes a Sling shift that is published, missing, not
  created by the app, or modified in Sling since the last push (compared
  against `push_result_snapshots`); pre-0013 pushes have no snapshot and are
  left alone. Keep it that way — the manager edits in Sling too.
- **Creating a webview window (the Sling login) must NOT happen on the UI
  thread on Windows.** `WebviewWindowBuilder::build()` blocks until WebView2's
  controller-ready notification arrives, and that only fires from the event
  loop's top-level message processing. Calling `build()` on the main thread —
  directly from a sync command, or via `run_on_main_thread` — nests it inside a
  user-event callback, the notification never arrives, and `build()` deadlocks:
  the window frame paints but the content stays blank. Fix: make the opening
  command `async` so it runs off the UI thread (see
  `open_sling_login_window` in `commands.rs`). WebKitGTK on Linux has no
  async-controller step, so this only bites on Windows.
- **On Windows, the app's `eprintln!`/stderr does NOT reliably reach the
  `tauri dev` terminal.** "No errors in the logs" can be misleading; to trace a
  Windows-only webview/runtime issue, log to a temp file instead.
