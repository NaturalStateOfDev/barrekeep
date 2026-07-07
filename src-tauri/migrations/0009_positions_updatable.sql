-- Migration 0009: make `positions` updatable during roster sync — drop the
-- FKs into positions and the UNIQUE(class_name) constraint.
--
-- Same DuckDB limitation as migrations 0003/0004, third occurrence: an UPDATE
-- that touches an indexed column (UNIQUE/PK), or any UPDATE on a row that is
-- referenced by a foreign key from another table, is executed internally as
-- DELETE+INSERT and fails with a constraint error. sync_roster() (runs on
-- every pull and on "Refresh from Sling") unconditionally executes
--   UPDATE positions SET class_name = ? WHERE sling_position_id = ?
-- class_name is UNIQUE (indexed), so the row is rewritten — which trips the
-- incoming FK from teacher_qualifications / proposal_shifts:
--   Constraint Error: Violates foreign key constraint because key
--   "sling_position_id: N" is still referenced by a foreign key in a
--   different table.
-- First reproduced in the wild by: generate a proposal, then pull.
--
-- DuckDB has no ALTER TABLE DROP CONSTRAINT, so the referencing tables and
-- positions itself are rebuilt (data preserved via INSERT ... SELECT;
-- seq_proposal_shifts keeps its high-water mark):
--   * teacher_qualifications — drop the FK to positions (teachers FK kept)
--   * proposal_shifts — drop the FK to positions and the always-NULL
--     self-FK on coteach_partner_shift_id (propose.py emits coteach_label
--     instead); proposals + teachers FKs kept
--   * positions — drop UNIQUE(class_name), keep the primary key
-- Integrity for the dropped constraints lives in application code: positions
-- are upserted by sync_roster before qualifications reference them, and
-- generate_proposal only emits positions read from the same database.

CREATE TABLE teacher_qualifications_new (
    sling_user_id      INTEGER NOT NULL REFERENCES teachers(sling_user_id),
    sling_position_id  INTEGER NOT NULL,
    is_blocklisted     BOOLEAN NOT NULL DEFAULT FALSE,
    blocklist_reason   VARCHAR,
    PRIMARY KEY (sling_user_id, sling_position_id)
);
INSERT INTO teacher_qualifications_new
SELECT sling_user_id, sling_position_id, is_blocklisted, blocklist_reason
FROM teacher_qualifications;
DROP TABLE teacher_qualifications;
ALTER TABLE teacher_qualifications_new RENAME TO teacher_qualifications;

CREATE TABLE proposal_shifts_new (
    id                       BIGINT PRIMARY KEY DEFAULT nextval('seq_proposal_shifts'),
    proposal_id              BIGINT NOT NULL REFERENCES proposals(id),
    shift_date               DATE NOT NULL,
    start_time               VARCHAR NOT NULL,
    end_time                 VARCHAR NOT NULL,
    sling_position_id        INTEGER NOT NULL,
    sling_user_id            INTEGER REFERENCES teachers(sling_user_id),
    generation_reason        VARCHAR NOT NULL,
    flag                     VARCHAR,
    is_coteach               BOOLEAN NOT NULL DEFAULT FALSE,
    coteach_partner_shift_id BIGINT,
    is_dropped               BOOLEAN NOT NULL DEFAULT FALSE,
    coteach_label            VARCHAR
);
INSERT INTO proposal_shifts_new
SELECT id, proposal_id, shift_date, start_time, end_time, sling_position_id,
       sling_user_id, generation_reason, flag, is_coteach,
       coteach_partner_shift_id, is_dropped, coteach_label
FROM proposal_shifts;
DROP TABLE proposal_shifts;
ALTER TABLE proposal_shifts_new RENAME TO proposal_shifts;
CREATE INDEX IF NOT EXISTS idx_prop_shifts_date
    ON proposal_shifts(proposal_id, shift_date, start_time);

CREATE TABLE positions_new (
    sling_position_id  INTEGER PRIMARY KEY,
    class_name         VARCHAR NOT NULL,
    duration_minutes   INTEGER NOT NULL DEFAULT 60,
    is_special         BOOLEAN NOT NULL DEFAULT FALSE,
    active             BOOLEAN NOT NULL DEFAULT TRUE
);
INSERT INTO positions_new
SELECT sling_position_id, class_name, duration_minutes, is_special, active
FROM positions;
DROP TABLE positions;
ALTER TABLE positions_new RENAME TO positions;
