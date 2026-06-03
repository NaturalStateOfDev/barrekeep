# Example Barre Studio — Scheduler

Desktop app for building monthly class schedules for a barre studio.

Setting up a dev machine, generating updater signing keys, or cutting a release? See **[SETUP.md](./SETUP.md)**.

## Quick start

```powershell
# One-time: install dependencies
npm install

# Run in dev mode (hot-reloads frontend, recompiles Rust on change)
npm run tauri dev

# Build the installer
npm run tauri build
# Output: src-tauri/target/release/bundle/msi/*.msi
```

> **First Rust build is slow.** The DuckDB Rust crate compiles DuckDB from C++
> source — expect 5–10 minutes the first time. Subsequent builds are fast.
> If you'd rather not wait, you can switch the `duckdb` dep in
> `src-tauri/Cargo.toml` from `bundled` to a system DuckDB once we know the
> install path is stable.

## What this app does

Pulls teacher availability from Sling, generates a monthly schedule using
rule-based + Claude-assisted proposal, lets you edit in a calendar UI, then
pushes back to Sling as planning-status (unpublished) shifts. Manager
publishes from Sling's web UI as the final step.

## Stack

- **Desktop shell:** Tauri 2 (Rust)
- **Frontend:** React 18 + TypeScript + Vite
- **Storage:** DuckDB (single-file embedded database)
- **Secrets:** Tauri Stronghold plugin (OS keychain) — _wired up later_
- **AI integration:** Anthropic SDK — _wired up later_
- **Sling integration:** Python sidecars in `scripts/`

## Repository layout

```
.
├── CLAUDE.md                 # Read this if you're working on the project
├── docs/                     # Architecture, Sling API notes, data model
├── data/                     # DuckDB file + CSV exports (gitignored)
├── prompts/                  # Claude prompts as versioned markdown
├── scripts/                  # Python utilities (Sling pull/push)
├── src/                      # React frontend
├── src-tauri/                # Rust shell + DuckDB integration
│   ├── migrations/           # SQL files, applied in order at startup
│   └── src/
│       ├── lib.rs            # Tauri entry — wires DB + commands
│       ├── db.rs             # DuckDB connection management
│       ├── migrations.rs     # Migration runner
│       ├── seed.rs           # First-run roster + positions data
│       └── commands.rs       # IPC commands the frontend calls
└── .claude/                  # Skills + subagents for Claude Code
```

See `docs/architecture.md` for the full layout with explanations.

## First-time setup

1. **Install Rust:** https://rustup.rs/
2. **Install Node.js 20+:** https://nodejs.org/
3. **Install Python 3.11+** (for the Sling sidecars): https://www.python.org/
4. **Install WebView2 runtime** (usually pre-installed on Windows 10+): https://developer.microsoft.com/en-us/microsoft-edge/webview2/
5. **Install Microsoft C++ Build Tools** (required for the DuckDB C++ compile): https://visualstudio.microsoft.com/visual-cpp-build-tools/
6. Clone, then `npm install`
7. Run `npm run tauri dev` — first run takes 5–10 min (DuckDB compile)

### Where the DB lives

`%LOCALAPPDATA%\com.barrekeep.app\scheduler.duckdb`

Delete that file to start fresh. Migrations + the seed (10 teachers + 7
class types) will recreate it on next launch.

## Development workflow

| Task | Where |
|---|---|
| Add a UI screen | `src/App.tsx` (or split out `src/components/`) |
| Add a Rust IPC command | `src-tauri/src/commands.rs` + register in `lib.rs` |
| Change the data model | `.claude/skills/schema-change/` |
| Touch Sling integration | `.claude/skills/sling-integration/` |
| Tune the algorithm | `.claude/skills/schedule-algorithm/` |

## Foundation status

**Done:**

- Tauri 2 shell that opens a window
- DuckDB embedded with forward-only migrations
- Roster + positions seeded on first run (Teacher A's qualifications populated;
  others' qualifications come from a Sling pull)
- React UI shell with Dashboard / Teachers / Class types views

**Not done (roadmap):**

- [ ] Sling pull command (Python sidecar via Tauri shell plugin)
- [ ] Sling token storage in Stronghold + UI to paste a fresh token
- [ ] Calendar editor (port the existing HTML widget)
- [ ] Algorithm v10 (the maintainer still has to drop the source somewhere)
- [ ] Anthropic client + prompt-driven review
- [ ] Push-to-Sling command + audit log surfacing
- [ ] Bundle icons (`src-tauri/icons/*`) — needed for `tauri build`, not for `tauri dev`

## Project history

This app started as a series of Python scripts and an in-browser widget. See
`docs/decisions/` for the trail of major architectural decisions.
