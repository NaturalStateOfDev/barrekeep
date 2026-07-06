# Barrekeep — barre studio scheduler

Barrekeep is a desktop scheduling tool for a single barre studio. The studio's
lead teacher uses it once a month to build the class schedule and push it to
Sling as planning-status (unpublished) shifts.

## What this app does

1. **Pull** teacher availability from Sling for the upcoming month
2. **Propose** a draft schedule using rule-based generation (with Claude as a tunable second opinion via the prompts library)
3. **Review** the draft in a calendar UI; edit teacher assignments, swap classes, flag conflicts
4. **Push** the approved draft to Sling as planning-status shifts (manager publishes from Sling's UI later)

The app is single-user, local-first, and runs on the user's Windows laptop. No
server, no cloud database.

## Stack

- **Shell:** Tauri (Rust-based, ships a small native installer)
- **Frontend:** React + TypeScript + Vite, plain CSS (no Tailwind)
- **Storage:** DuckDB embedded (single file at `data/scheduler.duckdb`)
- **Secrets:** Tauri Stronghold plugin (OS keychain, never on disk in plaintext)
- **AI:** Anthropic SDK for prompt-driven schedule analysis
- **Sling integration:** in-process Rust (`src-tauri/src/sling.rs` — pull, push,
  dedupe, rate limiting). Only `scripts/propose.py` (the schedule algorithm)
  still runs as a sidecar, fed a JSON payload over stdin.

## Repository layout

See `docs/architecture.md` for the full map. The summary:

- `src/` — React frontend
- `src-tauri/` — Rust shell + Tauri config (commands, migrations, seed)
- `scripts/` — Python utilities. `propose.py` is the schedule algorithm,
  invoked with a JSON payload over stdin. `sling_extract.py` and
  `push_to_sling.py` are Sling integration helpers.
- `prompts/` — Markdown files, one per Claude prompt (proposer, verifier).
  Versioned in git; read at runtime.
- `data/` — all local-only (gitignored). Holds the DuckDB database and any
  local pulls/fixtures. Nothing under `data/` ships.
- `docs/` — architecture, Sling API notes, data model
- `.claude/` — skills and subagents used by Claude Code when working on this repo

## Key constraints

- **Studio identifiers are runtime config, not compiled in.** The Sling org id,
  acting-user id, and home-location id live in the `studio_config` table
  (Settings → Studio configuration). A pull errors until they're set. The
  seeded roster is placeholder demo data; the real roster arrives via the pull.
- **Sling rate limits.** Aggressive. Push must be batched (10 shifts per batch,
  10s pause between batches, exponential backoff on 429). See `push_to_sling.py`.
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
5. **Never delete from `data/scheduler.duckdb` without a backup.** The schedule
   history is months of work.

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
- **DST transitions.** The studio observes US Central Time; the scripts
  currently send a fixed `-05:00` offset on date queries. Spring-forward /
  fall-back will need explicit timezone handling.
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
  dependabot PR.
