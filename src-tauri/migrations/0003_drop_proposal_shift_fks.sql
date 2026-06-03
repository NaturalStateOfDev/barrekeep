-- Migration 0003: drop FKs that reference proposal_shifts(id).
--
-- DuckDB's UPDATE on a row with an incoming FK reference fails with
-- "still referenced by a foreign key in a different table" because parts
-- of UPDATE are implemented as DELETE+INSERT internally. This breaks the
-- edit-teacher flow: INSERT into edits (which creates an FK reference)
-- followed by UPDATE on proposal_shifts (which trips the FK).
--
-- DuckDB has no ALTER TABLE DROP CONSTRAINT, so we rebuild both tables
-- without the FK. Data is preserved via INSERT ... SELECT. Sequences
-- (seq_edits, seq_push_results) keep their high-water marks since they
-- live independently of the tables.
--
-- The corresponding integrity check (proposal_shift_id must point to a
-- real shift) now lives in application code in src-tauri/src/commands.rs.

CREATE TABLE edits_new (
    id                BIGINT PRIMARY KEY DEFAULT nextval('seq_edits'),
    proposal_shift_id BIGINT NOT NULL,
    field             VARCHAR NOT NULL,
    old_value         VARCHAR,
    new_value         VARCHAR,
    reason            VARCHAR,
    edited_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    reverted          BOOLEAN NOT NULL DEFAULT FALSE
);
INSERT INTO edits_new
SELECT id, proposal_shift_id, field, old_value, new_value, reason, edited_at, reverted
FROM edits;
DROP TABLE edits;
ALTER TABLE edits_new RENAME TO edits;

CREATE TABLE push_results_new (
    id                BIGINT PRIMARY KEY DEFAULT nextval('seq_push_results'),
    push_id           BIGINT NOT NULL REFERENCES pushes(id),
    proposal_shift_id BIGINT NOT NULL,
    outcome           VARCHAR NOT NULL,
    sling_shift_id    VARCHAR,
    error_message     VARCHAR,
    attempted_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    retry_count       INTEGER NOT NULL DEFAULT 0
);
INSERT INTO push_results_new
SELECT id, push_id, proposal_shift_id, outcome, sling_shift_id, error_message, attempted_at, retry_count
FROM push_results;
DROP TABLE push_results;
ALTER TABLE push_results_new RENAME TO push_results;
