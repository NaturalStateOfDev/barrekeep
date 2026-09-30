-- Migration 0013: incremental push (sync) + availability refresh in place.
--
-- push_result_snapshots: what we actually sent to Sling for one push_results
-- row (created / updated / adopted). Incremental push compares Sling's
-- current shift against this snapshot before it updates or deletes anything:
-- if they differ, someone edited the shift in Sling and the app leaves it
-- alone. push_results itself only records proposal_shift_id + sling_shift_id,
-- which can't express this: a co-teach row is TWO Sling shifts under one
-- proposal_shift_id, and the proposal shift may have been edited since.
--
-- A SIDE TABLE rather than ALTER TABLE push_results ADD COLUMN: push_results
-- holds an FK into pushes, and DuckDB's ALTER on FK-linked tables is exactly
-- the kind of constraint machinery CLAUDE.md warns about (0003/0004/0009).
-- Rows are insert-only, never UPDATEd. PK only, no FKs in or out.
--
-- Pre-0013 push_results rows have no snapshot. Sync treats them as
-- "can't verify it's unmodified": they still count as unchanged when Sling
-- matches the draft exactly, but are never updated or deleted automatically.
-- No backfill — a guessed snapshot would defeat the safety check.
--
-- draft_checks: when a draft was last re-validated against the month's
-- latest pulled availability (check_draft_conflicts). A draft is stale only
-- if the latest pull is newer than BOTH its generation and its last check,
-- so refreshing availability in place + checking clears the stale banner
-- without regenerating (and losing edits). Written with INSERT OR REPLACE
-- (app_settings pattern). PK only, no FKs.

CREATE TABLE IF NOT EXISTS push_result_snapshots (
    push_result_id    BIGINT PRIMARY KEY,
    sling_user_id     BIGINT NOT NULL,
    sling_position_id BIGINT NOT NULL,
    shift_date        VARCHAR NOT NULL,   -- 'YYYY-MM-DD' (studio local)
    start_time        VARCHAR NOT NULL,   -- 'HH:MM' (studio local)
    end_time          VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS draft_checks (
    proposal_id BIGINT PRIMARY KEY,
    checked_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
