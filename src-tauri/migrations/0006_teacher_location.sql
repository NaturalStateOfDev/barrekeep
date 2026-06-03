-- Migration 0006: teacher location + the Sling candidate roster.
-- Forward-only. Idempotent so re-running on an existing DB is safe.

ALTER TABLE teachers ADD COLUMN IF NOT EXISTS locations VARCHAR;

-- Mirror of Sling's roster (limited to candidates we could plausibly
-- add to our scheduling roster: active + holds a teaching position +
-- tagged to the home location location). Wiped + repopulated on each pull.
-- Users already in `teachers` are excluded — this table only contains
-- the "addable" delta.
CREATE TABLE IF NOT EXISTS sling_candidates (
    sling_user_id INTEGER PRIMARY KEY,
    display_name  VARCHAR NOT NULL,
    active        BOOLEAN NOT NULL,
    locations     VARCHAR,
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
