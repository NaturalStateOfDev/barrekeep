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

### `sling_candidates`

Mirror of Sling's roster, filtered to active + holds a teaching position +
tagged to the home location location. Wiped + repopulated on every pull. The
teachers page's "Add teacher from Sling" picker reads from here, minus
users already in `teachers`.

```sql
CREATE TABLE sling_candidates (
  sling_user_id INTEGER PRIMARY KEY,
  display_name  VARCHAR NOT NULL,
  active        BOOLEAN NOT NULL,
  locations     VARCHAR,
  last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### `studio_config`

Singleton (always `id = 1`) holding the studio's Sling identifiers. These were
formerly compiled-in constants; they now live here so the shipped/public binary
carries no real org identity and each install configures its own studio at
runtime (Settings → Studio configuration). Seeded with `0` placeholders; a pull
errors with a "configure your studio" message until real values are entered.

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
  is_current         BOOLEAN NOT NULL DEFAULT FALSE  -- only one current per target_month
);
CREATE SEQUENCE seq_proposals;
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
  prompt_id            BIGINT NOT NULL REFERENCES prompts(id),
  proposal_id          BIGINT REFERENCES proposals(id),
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
  outcome              VARCHAR NOT NULL,  -- 'created' | 'failed' | 'skipped'
  sling_shift_id       VARCHAR,  -- if created
  error_message        VARCHAR,  -- if failed
  attempted_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  retry_count          INTEGER NOT NULL DEFAULT 0
);
CREATE SEQUENCE seq_push_results;
```

## Common queries

```sql
-- Current proposal for a target month
SELECT * FROM proposals
WHERE target_month = '2026-07' AND is_current = TRUE;

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

## Reference data (seeded on first run)

See `src/lib/seed.ts` for the initial values:

- 10 teachers (current roster)
- 7 positions (Empower, Focus, Breaking Down the Barre, Align, Classic, Define, Reform)
- All 35 (teacher × position) qualifications from the Sling group memberships
- One blocklist row: Teacher E × Reform (manager hasn't approved them yet)

The seed runs only if `teachers` is empty. After that, edits are user-driven.
