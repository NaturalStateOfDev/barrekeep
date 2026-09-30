# Data model

DuckDB schema for `data/scheduler.duckdb`. The DDL below is the source of truth; `src/lib/migrations.ts` runs equivalent statements at app startup.

## Conventions

- All ids are `INTEGER` unless they come from Sling, which uses string-form bigints (`VARCHAR`).
- All timestamps are `TIMESTAMPTZ`.
- All money values are `DECIMAL(10, 4)` so we don't lose fractional cents on Anthropic API costs.
- Soft-deletes via `deleted_at TIMESTAMPTZ NULL` rather than hard deletes, so undo is always possible.

## Tables

### `teachers`

Roster as of latest sync. Source of truth: Sling, with optional manager overrides.

```sql
CREATE TABLE teachers (
  sling_user_id      INTEGER PRIMARY KEY,
  display_name       VARCHAR NOT NULL,
  weekly_target      INTEGER NOT NULL,
  weekly_max         INTEGER NOT NULL,
  is_lead            BOOLEAN NOT NULL DEFAULT FALSE,
  ranking_weight     DOUBLE NOT NULL DEFAULT 1.0,
  variety_multiplier DOUBLE NOT NULL DEFAULT 1.0,
  active             BOOLEAN NOT NULL DEFAULT TRUE,
  notes              VARCHAR,
  locations          VARCHAR,                          -- comma-joined Sling location names, derived from groupIds at pull time
  updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### `month_pulls`

One row per pulled month — when and what the last pull for that month brought
in. `get_proposal` compares `pulled_at` against `generated_at` to flag stale
proposals. (The `sling_candidates` table that used to live here was dropped in
migration 0008 — the roster now syncs directly into `teachers`.)

```sql
CREATE TABLE month_pulls (
  target_month         VARCHAR PRIMARY KEY,  -- 'YYYY-MM'
  pulled_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  user_count           INTEGER NOT NULL,
  qual_count           INTEGER NOT NULL,
  availability_count   INTEGER NOT NULL,
  external_shift_count INTEGER NOT NULL
);
```

### `external_sling_shifts`

Shifts that already exist in Sling (pulled per month, plus trailing history
used by the proposer's slot template). Replaced per target month on each pull.

```sql
CREATE TABLE external_sling_shifts (
  sling_shift_id    BIGINT PRIMARY KEY,
  target_month      VARCHAR NOT NULL,   -- denormalized for fast WHERE
  shift_date        DATE NOT NULL,
  start_time        VARCHAR NOT NULL,   -- 'HH:MM'
  end_time          VARCHAR NOT NULL,
  sling_user_id     INTEGER,            -- nullable: unassigned shifts exist
  sling_position_id INTEGER NOT NULL,
  status            VARCHAR NOT NULL,   -- 'planning' | 'published'
  pulled_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_ext_shift_month ON external_sling_shifts(target_month);
```

### `studio_config`

Singleton (always `id = 1`) holding the studio's Sling identifiers. These were
formerly compiled-in constants; they now live here so the shipped/public binary
carries no real org identity and each install configures its own studio at
runtime (Settings → Studio configuration). Seeded with `0` placeholders; a pull
errors with a "configure your studio" message until real values are entered.

After a Sling login (and at startup while the row is incomplete) the app
detects the values from the logged-in user (`auto_detect_studio_config`,
rule in `src-tauri/src/studio_setup.rs`): an unset config with exactly one
candidate per field is saved automatically; several candidates open a picker;
a complete config is never overwritten — a disagreeing login only raises a
"doesn't match" banner. Writes are compare-before-write.

```sql
CREATE TABLE studio_config (
  id               INTEGER PRIMARY KEY,   -- always 1 (singleton)
  org_id           BIGINT NOT NULL DEFAULT 0,   -- Sling organization id
  acting_user_id   BIGINT NOT NULL DEFAULT 0,   -- admin user whose calendar feed we read
  home_location_id BIGINT NOT NULL DEFAULT 0,   -- the studio location to keep (others filtered out)
  updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

See `docs/sling-api.md` for how these map to the calendar endpoint URL and the
location filter.

### `positions`

Class types and Sling position mapping.

```sql
CREATE TABLE positions (
  sling_position_id  INTEGER PRIMARY KEY,
  class_name         VARCHAR NOT NULL,  -- UNIQUE dropped in migration 0009 (DuckDB indexed-update limitation)
  duration_minutes   INTEGER NOT NULL DEFAULT 60,
  is_special         BOOLEAN NOT NULL DEFAULT FALSE,  -- Focus, etc.
  active             BOOLEAN NOT NULL DEFAULT TRUE
);
```

> **Why no UNIQUE on `class_name`, and no FKs into this table** (migration
> 0009): DuckDB executes an UPDATE that touches an indexed column as
> DELETE+INSERT, which fails with "still referenced by a foreign key in a
> different table" whenever `teacher_qualifications` / `proposal_shifts` rows
> reference the position. `sync_roster()` must update `class_name` on every
> pull, so both the UNIQUE constraint and the incoming FKs had to go —
> the same limitation that motivated migrations 0003 and 0004. Integrity now
> lives in application code: positions are upserted before anything
> references them.

### `teacher_qualifications`

Many-to-many: which teachers can teach which classes (from Sling group membership).

```sql
CREATE TABLE teacher_qualifications (
  sling_user_id      INTEGER NOT NULL REFERENCES teachers(sling_user_id),
  sling_position_id  INTEGER NOT NULL,  -- FK to positions dropped in migration 0009
  is_blocklisted     BOOLEAN NOT NULL DEFAULT FALSE,  -- manager override
  blocklist_reason   VARCHAR,
  PRIMARY KEY (sling_user_id, sling_position_id)
);
```

### `availability_blocks`

Pulled from Sling per month. Despite the name, these are BLOCKED times (Sling's `availability` event type is backward).

```sql
CREATE TABLE availability_blocks (
  id                 BIGINT PRIMARY KEY DEFAULT nextval('seq_availability'),
  sling_user_id      INTEGER NOT NULL REFERENCES teachers(sling_user_id),
  source             VARCHAR NOT NULL,  -- 'leave' | 'recurring' | 'manual'
  starts_at          TIMESTAMPTZ NOT NULL,
  ends_at            TIMESTAMPTZ NOT NULL,
  pulled_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE SEQUENCE seq_availability;
CREATE INDEX idx_avail_user_time ON availability_blocks(sling_user_id, starts_at, ends_at);
```

### `proposals`

One row per generation run. The actual schedule rows live in `proposal_shifts`.

```sql
CREATE TABLE proposals (
  id                 BIGINT PRIMARY KEY DEFAULT nextval('seq_proposals'),
  target_month       VARCHAR NOT NULL,  -- 'YYYY-MM'
  algorithm_version  VARCHAR NOT NULL,  -- 'v9', 'v10', etc.
  parameters         JSON NOT NULL,     -- variety_penalty, ranking_weights, etc.
  generated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  notes              VARCHAR,
  is_current         BOOLEAN NOT NULL DEFAULT FALSE  -- newest GENERATED draft per month; kept for
                                                      -- backward compat — "which draft gets pushed"
                                                      -- lives in month_push_candidate (migration 0012)
);
CREATE SEQUENCE seq_proposals;
```

A month can hold several **drafts** side by side (generated or duplicated —
e.g. a "what if Jane always teaches Tue 8:45" variant). Each proposal row is
one draft. Draft metadata and the push choice live in side tables (migration
0012) so `proposals` itself is never ALTERed or UPDATEd for them.

### `proposal_drafts`

Draft metadata, one row per proposal (migration 0012; backfilled "Draft 1",
"Draft 2", … per month in creation order). PK-only, no FKs in or out, so the
`name`/`archived` UPDATEs from rename/archive are safe under the DuckDB rules.

```sql
CREATE TABLE proposal_drafts (
  proposal_id        BIGINT PRIMARY KEY,  -- logically references proposals(id)
  name               VARCHAR NOT NULL,
  parent_proposal_id BIGINT,              -- set when created by "Duplicate"
  created_from       VARCHAR NOT NULL DEFAULT 'generate',  -- 'generate' | 'duplicate'
  archived           BOOLEAN NOT NULL DEFAULT FALSE,       -- hidden by default; never the push draft
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Duplicating a draft copies the `proposals` row (keeping `generated_at`, so
staleness vs the last pull carries over; `is_current = FALSE`) and every
`proposal_shifts` row with fresh ids, remapping `coteach_partner_shift_id`.
Edit history (`edits`) and push history are NOT copied.

### `month_push_candidate`

The single source of truth for "which draft is THE draft" for a month: the
one Push sends to Sling. Push refuses any other draft (Sling dedupe matches
date+time+teacher+position, so pushing a second draft would ADD its differing
shifts on top of the first). Written with `INSERT OR REPLACE` (same pattern
as `app_settings`). Generate sets it only when the month has none; the user
changes it with "Use for push". Backfilled from the pushed / `is_current` /
newest proposal per month.

```sql
CREATE TABLE month_push_candidate (
  target_month VARCHAR PRIMARY KEY,
  proposal_id  BIGINT NOT NULL,  -- logically references proposals(id)
  set_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### `claude_run_targets`

Links one Claude edit prompt to every draft it was applied to (append-only).
A prompt sent to N drafts makes N `claude_runs` rows (one API call per draft —
shift ids differ); `claude_run_id` here is the FIRST run of that prompt, with
one row per targeted draft. Single-draft runs link to themselves. Backfilled
from `claude_runs.proposal_id`.

```sql
CREATE TABLE claude_run_targets (
  claude_run_id BIGINT NOT NULL,  -- the prompt's first claude_runs.id
  proposal_id   BIGINT NOT NULL,
  PRIMARY KEY (claude_run_id, proposal_id)
);
```

### `proposal_shifts`

Generated assignments. One row per slot. Co-teach rows have `is_coteach = TRUE`; propose.py emits the pairing as a `coteach_label` string ("Teacher A + Teacher E") on a single row (migration 0002) rather than sibling rows linked via `coteach_partner_shift_id`.

```sql
CREATE TABLE proposal_shifts (
  id                       BIGINT PRIMARY KEY DEFAULT nextval('seq_proposal_shifts'),
  proposal_id              BIGINT NOT NULL REFERENCES proposals(id),
  shift_date               DATE NOT NULL,
  start_time               VARCHAR NOT NULL,  -- 'HH:MM'
  end_time                 VARCHAR NOT NULL,
  sling_position_id        INTEGER NOT NULL,  -- FK to positions dropped in migration 0009
  sling_user_id            INTEGER REFERENCES teachers(sling_user_id),  -- NULL = dropped
  generation_reason        VARCHAR NOT NULL,  -- 'primary, under target', 'format-flex', etc.
  flag                     VARCHAR,           -- 'TEACHER_X - VERIFY', 'NEW 7AM SLOT', etc.
  is_coteach               BOOLEAN NOT NULL DEFAULT FALSE,
  coteach_partner_shift_id BIGINT,  -- self-FK dropped in migration 0009; always NULL today
  is_dropped               BOOLEAN NOT NULL DEFAULT FALSE,
  coteach_label            VARCHAR  -- added in migration 0002
);
CREATE SEQUENCE seq_proposal_shifts;
CREATE INDEX idx_prop_shifts_date ON proposal_shifts(proposal_id, shift_date, start_time);
```

### `edits`

Manual user edits to a proposal. Each edit captures before-state for full undo.

```sql
CREATE TABLE edits (
  id                  BIGINT PRIMARY KEY DEFAULT nextval('seq_edits'),
  proposal_shift_id   BIGINT NOT NULL,  -- logically references proposal_shifts(id), see note
  field               VARCHAR NOT NULL,  -- 'sling_user_id' | 'sling_position_id' | etc.
  old_value           VARCHAR,
  new_value           VARCHAR,
  reason              VARCHAR,
  edited_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
  reverted            BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE SEQUENCE seq_edits;
```

**Note: `proposal_shift_id` is intentionally not a foreign key.** DuckDB's
UPDATE on a row that has incoming FK references fails with "still referenced
by foreign key" — UPDATE is implemented as DELETE+INSERT internally, which
trips the FK check. Recording an edit *and* updating the shift in one
transaction is exactly that pattern. The integrity invariant is enforced in
application code (`edit_proposal_shift_teacher` in `commands.rs`). The same
applies to `push_results.proposal_shift_id` below.

### `prompts`

Mirror of `prompts/*.md` files. App reads from disk on startup; if a file is newer than the DB row, the row is updated and a new version is created.

```sql
CREATE TABLE prompts (
  id              BIGINT PRIMARY KEY DEFAULT nextval('seq_prompts'),
  name            VARCHAR NOT NULL,
  version         INTEGER NOT NULL,
  body            VARCHAR NOT NULL,
  source_file     VARCHAR,  -- path under prompts/
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (name, version)
);
CREATE SEQUENCE seq_prompts;
```

### `claude_runs`

Audit log of every Anthropic API call: which prompt, what input, what came back, what it cost.

```sql
CREATE TABLE claude_runs (
  id                   BIGINT PRIMARY KEY DEFAULT nextval('seq_claude_runs'),
  prompt_id            BIGINT,   -- FKs dropped in migration 0004; nullable until prompt syncing exists
  proposal_id          BIGINT,   -- logically references proposals(id), enforced in app code
  model               VARCHAR NOT NULL,    -- 'claude-opus-4-7' etc.
  input_tokens         INTEGER NOT NULL,
  output_tokens        INTEGER NOT NULL,
  input_text           VARCHAR NOT NULL,
  output_text          VARCHAR NOT NULL,
  cost_usd             DECIMAL(10, 4) NOT NULL,
  duration_ms          INTEGER NOT NULL,
  ran_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE SEQUENCE seq_claude_runs;
```

### `pushes`

One row per push-to-Sling run.

```sql
CREATE TABLE pushes (
  id                   BIGINT PRIMARY KEY DEFAULT nextval('seq_pushes'),
  proposal_id          BIGINT NOT NULL REFERENCES proposals(id),
  started_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at          TIMESTAMPTZ,
  shifts_attempted     INTEGER NOT NULL DEFAULT 0,
  shifts_succeeded     INTEGER NOT NULL DEFAULT 0,
  shifts_failed        INTEGER NOT NULL DEFAULT 0,
  shifts_skipped       INTEGER NOT NULL DEFAULT 0  -- dedupe matches
);
CREATE SEQUENCE seq_pushes;
```

### `push_results`

Per-shift outcome of each push.

```sql
CREATE TABLE push_results (
  id                   BIGINT PRIMARY KEY DEFAULT nextval('seq_push_results'),
  push_id              BIGINT NOT NULL REFERENCES pushes(id),
  proposal_shift_id    BIGINT NOT NULL,  -- logically references proposal_shifts(id), see edits note
  outcome              VARCHAR NOT NULL,  -- see outcomes below
  sling_shift_id       VARCHAR,  -- if created
  error_message        VARCHAR,  -- if failed
  attempted_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  retry_count          INTEGER NOT NULL DEFAULT 0
);
CREATE SEQUENCE seq_push_results;
```

Outcomes: `created` (first push), `updated` (re-created after a draft change —
new `sling_shift_id`), `adopted` (another draft's identical shift now tracked
for this draft; no Sling call), `deleted`, `skipped_missing` (gone from Sling),
`skipped_conflict` (published / edited in Sling / pre-0013 — left alone),
`failed`. The latest `created`/`updated`/`adopted`/`deleted`/`skipped_missing`
row per `sling_shift_id` decides whether the app still owns that Sling shift
(see `src-tauri/src/push_sync.rs`).

### `push_result_snapshots` (migration 0013)

What was actually sent to Sling for one `push_results` row. Insert-only; PK
only, no FKs (side table rather than new `push_results` columns — see the
migration header). Pre-0013 rows have none; a push baselines one (an
`adopted` row + snapshot of Sling's state) only when Sling shows it planning
and equal to its draft shift — otherwise sync never changes it.

```sql
CREATE TABLE push_result_snapshots (
  push_result_id    BIGINT PRIMARY KEY,
  sling_user_id     BIGINT NOT NULL,
  sling_position_id BIGINT NOT NULL,
  shift_date        VARCHAR NOT NULL,   -- 'YYYY-MM-DD', studio local
  start_time        VARCHAR NOT NULL,   -- 'HH:MM'
  end_time          VARCHAR NOT NULL
);
```

### `draft_checks` (migration 0013)

Last time a draft was re-validated against the month's pulled data
(`check_draft_conflicts`). A draft is stale iff
`month_pulls.pulled_at > max(generated_at, checked_at)`. Written with
`INSERT OR REPLACE`; copied on duplicate.

```sql
CREATE TABLE draft_checks (
  proposal_id BIGINT PRIMARY KEY,
  checked_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

## Common queries

```sql
-- The push draft for a target month
SELECT p.*, d.name FROM month_push_candidate m
JOIN proposals p ON p.id = m.proposal_id
LEFT JOIN proposal_drafts d ON d.proposal_id = p.id
WHERE m.target_month = '2026-07';

-- Full schedule for a proposal, with teacher and class names joined
SELECT ps.shift_date, ps.start_time, ps.end_time,
       p.class_name, t.display_name AS teacher,
       ps.flag, ps.generation_reason
FROM proposal_shifts ps
LEFT JOIN positions p ON ps.sling_position_id = p.sling_position_id
LEFT JOIN teachers t ON ps.sling_user_id = t.sling_user_id
WHERE ps.proposal_id = ?
ORDER BY ps.shift_date, ps.start_time;

-- Weekly load per teacher for a proposal
SELECT t.display_name,
       date_trunc('week', ps.shift_date) AS week,
       count(*) AS classes
FROM proposal_shifts ps
JOIN teachers t ON ps.sling_user_id = t.sling_user_id
WHERE ps.proposal_id = ? AND NOT ps.is_dropped
GROUP BY t.display_name, week
ORDER BY t.display_name, week;

-- Total Anthropic spend this month
SELECT sum(cost_usd) AS total_usd
FROM claude_runs
WHERE date_trunc('month', ran_at) = date_trunc('month', now());
```

## Reference data

There is no seed data (`src-tauri/src/seed.rs` is intentionally a no-op, and
migration 0008 purged the placeholder demo roster older installs carried).
A fresh install starts empty; the roster, positions, and qualifications
arrive via the Sling pull or the Teachers page's "Refresh from Sling".
Weekly caps default to target 4 / max 5 on import and are edited in-app;
position duration/special flags are edited in-app after import.


### `app_settings`

Tiny key-value store for user preferences that belong in the DB (keys include
`claude_model` and `time_format` = `12h` | `24h`, display-only, default
`12h`). PK-only, no extra indexes, no FKs — `INSERT OR REPLACE`
upserts are safe under the DuckDB constraint rules.

```sql
CREATE TABLE app_settings (
  key        VARCHAR PRIMARY KEY,
  value      VARCHAR NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### `algorithm_versions`

Append-only history of adopted algorithm versions (migration 0010). Version 9
is the implicit baseline: the shipped `scripts/propose.py` with empty rules.
Adopted versions start at 10. `rules` is a full snapshot, not a delta.
`script_file` NULL = run the shipped baseline script; otherwise a file under
`<app_local_data>/algorithms/` (resolution also checks `algorithms/archive/`).
Rows are inserted on explicit user adoption and never UPDATEd; "last used"
per version is derived from `proposals.algorithm_version` + `generated_at`.

```sql
CREATE TABLE algorithm_versions (
  version       INTEGER PRIMARY KEY,
  description   VARCHAR NOT NULL,
  rules         JSON NOT NULL,
  script_file   VARCHAR,
  created_by    VARCHAR NOT NULL,      -- 'claude' | 'user'
  claude_run_id BIGINT,                -- provenance into claude_runs (app-enforced)
  adopted_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  baseline_sha256 VARCHAR              -- migration 0011: sha256 of the shipped propose.py at adoption
);
```

A rules-only adoption reuses the **active** version's `script_file` (and its
`baseline_sha256`) — it never silently falls back to the shipped script, so an
adopted code change survives later rule tweaks. Several rows may therefore
point at the same script file.

**Active version.** Which version `generate_proposal` runs is stored in
`app_settings` under `active_algorithm_version` (an integer; `9` = the shipped
baseline). Adopting a version makes it active; "Make active" in the Algorithm
card rolls back/forward without touching `algorithm_versions` rows. When the
key is unset or names a version that no longer exists, the newest row wins.
