-- Migration 0005: tables backing the live Sling pull.
-- Forward-only. Empty tables are fine; first pull populates them.

CREATE SEQUENCE IF NOT EXISTS seq_external_sling_shifts;

CREATE TABLE IF NOT EXISTS month_pulls (
    target_month         VARCHAR PRIMARY KEY,  -- 'YYYY-MM'
    pulled_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    user_count           INTEGER NOT NULL,
    qual_count           INTEGER NOT NULL,
    availability_count   INTEGER NOT NULL,
    external_shift_count INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS external_sling_shifts (
    sling_shift_id    BIGINT PRIMARY KEY,
    target_month      VARCHAR NOT NULL,         -- denormalized for fast WHERE
    shift_date        DATE NOT NULL,
    start_time        VARCHAR NOT NULL,         -- 'HH:MM'
    end_time          VARCHAR NOT NULL,
    sling_user_id     INTEGER,                  -- nullable: unassigned shifts exist
    sling_position_id INTEGER NOT NULL,
    status            VARCHAR NOT NULL,         -- 'planning' | 'published'
    pulled_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ext_shift_month ON external_sling_shifts(target_month);
