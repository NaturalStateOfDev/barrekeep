# Barrekeep

A desktop app for building a barre studio's monthly class schedule.

It pulls teacher availability and recent history from [Sling](https://getsling.com),
proposes a draft month with a rule-based algorithm (optionally critiqued by
Claude), lets you review and edit it in a calendar UI, and pushes the approved
draft back to Sling as **planning-status** (unpublished) shifts — a manager
publishes from Sling's web UI as the final step.

Single-user, local-first. No server, no cloud database. Ships with no data:
the roster, positions and qualifications arrive from your Sling account, and
the studio is configured at runtime.

> Setting up a dev machine, generating updater signing keys, or cutting a
> release? See **[SETUP.md](./SETUP.md)**.

## What works

- **Pull from Sling** — roster, qualifications (from Sling position groups),
  availability blocks, and existing/historical shifts for a chosen month.
- **Generate** a draft schedule for any month (rule-based; ranking learned
  from recent history, candidate pool from Sling positions).
- **Review** in a month-grid calendar with an issue queue (unassigned slots,
  over-cap teachers, qualification conflicts, leave conflicts) and one-click
  fixes.
- **Edit** teacher assignments and weekly caps inline.
- **Drafts** — several per month, compare two side by side, pick one as the
  month's push draft.
- **Push / sync** the push draft to Sling (batched + rate-limit-aware), as
  planning status only. Re-pushing updates only shifts the app created and
  nobody has touched in Sling since.
- **Refresh availability** — re-pull Sling and re-check an existing draft
  without regenerating it.
- **In-app Sling login** (captures the bearer token) or paste one manually;
  tokens persist in the OS keychain (Stronghold).
- **Auto-update** — installs pick up new signed releases from GitHub.

## Stack

- **Shell:** Tauri 2 (Rust) — small native installer, WebView2/WebKitGTK
- **Frontend:** React 18 + TypeScript + Vite, plain CSS
- **Storage:** DuckDB (single-file embedded database)
- **Secrets:** Tauri Stronghold plugin (OS keychain)
- **AI:** Anthropic SDK (optional, for prompt-driven schedule review)
- **Sling integration:** in-process Rust (`ureq`) — `sling.rs`, `push_sync.rs`
- **Algorithm:** `scripts/propose.py`, run as a Python subprocess

## Quick start

```bash
npm ci
npm run tauri dev          # hot-reload frontend, recompile Rust on change
npm run tauri build        # -> src-tauri/target/release/bundle/
```

> **First Rust build is slow** (~5–10 min): the `duckdb` crate compiles DuckDB
> from C++ source. Subsequent builds are fast.

### Prerequisites

- [Rust](https://rustup.rs/) 1.88 (pinned in `rust-toolchain.toml`),
  [Node.js 22](https://nodejs.org/), [Python 3.11+](https://www.python.org/)
- Windows: [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/)
  + [MSVC C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).
  Linux: WebKitGTK/GTK dev packages and `mold` — see [SETUP.md](./SETUP.md)
  for apt and pacman package lists.
- The installed app needs Python 3.11+ on PATH too (it runs `propose.py`);
  Settings → Python checks it. See SETUP.md → Studio PC requirements.

### First-run configuration

On first launch the app starts empty and creates its
database at `%LOCALAPPDATA%\com.barrekeep.app\scheduler.duckdb` (Windows) /
`~/.local/share/com.barrekeep.app/` (Linux). Then, in **Settings**:

1. **Sling token** — log in via the in-app browser, or paste a bearer token.
2. **Studio configuration** — after a Sling login the app detects your Sling
   **org id**, **acting-user id** (whose calendar feed is read) and **home
   location id**, saving them when there's one obvious choice and asking
   otherwise. You can also enter them by hand (they appear in the calendar
   request URL in a Sling DevTools session). They're stored locally only;
   nothing studio-specific is compiled into the app.
3. (Optional) **Anthropic key** — to enable Claude-assisted review.

Then pick a month → **Pull from Sling** → **Generate** → review → **Push**.

## Repository layout

```
.
├── CLAUDE.md          # orientation for working on the project
├── docs/              # architecture, Sling API notes, data model
├── prompts/           # Claude prompts as versioned markdown
├── scripts/           # propose.py (the algorithm), tests, version bump;
│                      # legacy/ = retired Sling scripts, reference only
├── src/               # React frontend
├── src-tauri/         # Rust shell + DuckDB
│   ├── migrations/    # forward-only SQL, applied at startup
│   └── src/           # commands.rs, sling.rs, push_sync.rs, drafts.rs, ...
└── .claude/           # skills + subagents for Claude Code
```

## Known limitations & roadmap

This started as a tool for one studio and is being generalized. Current edges:

- **Class-type / position mapping is hardcoded** in `scripts/propose.py`
  (`POSITION_NAMES` / `CLASS_POSITION_IDS`) to one studio's Sling position ids
  and class names. The Sling pull already fetches each studio's position
  groups, so the next step is to pass that mapping through the payload instead
  of hardcoding it — making the algorithm truly studio-agnostic (mirrors how
  org/location ids were already moved to runtime config).
- **Standalone `propose.py` is dev-only.** Run from the app, the target month
  is parameterized (`--target-month`, driven by the month selector) and data
  comes from the live pull — so it's run fresh each month by design. The
  hardcoded month default and fixture file paths in the script only affect
  running it standalone without the app.
- **Per-studio scheduling rules** (priority slots, blocklists, hard
  assignments, month-specific overrides) ship empty/generic; they're currently
  code-level extension points rather than UI-configurable.

## License

_Not yet licensed — all rights reserved until a license is added._
