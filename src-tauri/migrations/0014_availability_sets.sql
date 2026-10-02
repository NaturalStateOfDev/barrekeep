-- Migration 0014: recurring availability sets, studio hours, and computed
-- per-teacher availability windows.
--
-- WHY: teachers enter their recurring UNAVAILABILITY in Sling as
-- "availability sets" (GET /availability?userId=…). The Rust pull only read
-- /calendar, so those recurrences never arrived and the scheduler treated the
-- teachers as free (only one-off blocks made it into availability_blocks).
--
-- sling_availability_sets: one row per set as Sling returned it — raw JSON
-- plus the few parsed fields the app shows. Replaced per teacher on every
-- pull (DELETE + INSERT; rows are never UPDATEd). `interval_days` is the
-- interpreted recurrence step (7 = weekly, 14 = every two weeks); NULL means
-- the app could NOT interpret the set (unknown `interval` wire format, bad
-- dates) — it produces no blocks and the UI warns about it. `pending` = the
-- set has no `approved` timestamp; it still blocks (safer for scheduling)
-- but is labelled "pending approval". Surrogate PK only: Sling's set id is
-- kept as text (it may be absent, numeric or a string). No FKs, no UNIQUE.
--
-- The expanded occurrences land in the existing availability_blocks table
-- with two new `source` values — 'availability_set' and
-- 'availability_set_pending' — beside 'availability' and 'leave'. No ALTER
-- on availability_blocks: it holds an FK into teachers, and ALTERs on
-- FK-linked tables are the constraint machinery CLAUDE.md warns about.
--
-- studio_hours: per-weekday open/close the lead teacher sets in Settings
-- (weekday 0 = Monday … 6 = Sunday). An EMPTY table means "not set": hours
-- are then derived from the class slots themselves. Saved by DELETE + INSERT.
--
-- teacher_availability_windows: the computed AVAILABLE windows per teacher
-- per date (studio hours, widened to any class slot outside them, minus every
-- block). Derived data, rewritten per month (DELETE + INSERT) on every pull,
-- refresh and studio-hours change. No PK, no FKs — nothing references it and
-- it can always be rebuilt from availability_blocks.
--
-- Everything is additive and IF NOT EXISTS, so re-running is a no-op.

CREATE SEQUENCE IF NOT EXISTS seq_sling_availability_sets;

CREATE TABLE IF NOT EXISTS sling_availability_sets (
    id                 BIGINT PRIMARY KEY DEFAULT nextval('seq_sling_availability_sets'),
    sling_set_id       VARCHAR,            -- Sling's id as text; NULL if absent
    sling_user_id      INTEGER NOT NULL,
    name               VARCHAR,
    starts_on          VARCHAR,            -- set `start` as Sling sent it
    until              VARCHAR,            -- set `until`; NULL = recurs forever
    interval_raw       VARCHAR,            -- `interval` exactly as sent (JSON text)
    interval_days      INTEGER,            -- interpreted step; NULL = uninterpreted
    approved_at        VARCHAR,            -- `approved`; NULL = pending approval
    pending            BOOLEAN NOT NULL DEFAULT FALSE,
    availability_count INTEGER NOT NULL DEFAULT 0,
    problem            VARCHAR,            -- why it couldn't be (fully) interpreted
    raw_json           VARCHAR NOT NULL,
    pulled_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_avail_sets_user ON sling_availability_sets(sling_user_id);

CREATE TABLE IF NOT EXISTS studio_hours (
    weekday    INTEGER PRIMARY KEY,        -- 0 = Monday … 6 = Sunday
    closed     BOOLEAN NOT NULL DEFAULT FALSE,
    open_time  VARCHAR,                    -- 'HH:MM' studio local; NULL when closed
    close_time VARCHAR
);

CREATE TABLE IF NOT EXISTS teacher_availability_windows (
    target_month  VARCHAR NOT NULL,        -- 'YYYY-MM'
    sling_user_id INTEGER NOT NULL,
    window_date   VARCHAR NOT NULL,        -- 'YYYY-MM-DD' studio local
    start_time    VARCHAR NOT NULL,        -- 'HH:MM' studio local
    end_time      VARCHAR NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_avail_windows_month ON teacher_availability_windows(target_month);
