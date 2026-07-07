-- Migration 0010: Claude proposal editor backing tables.
--
-- app_settings: tiny key-value store for user preferences that belong in
-- the DB (first key: 'claude_model'). PK-only, no extra indexes, no FKs —
-- INSERT OR REPLACE upserts are safe under the DuckDB rules (CLAUDE.md).
--
-- algorithm_versions: append-only history of adopted algorithm versions.
-- version 9 is the implicit baseline (shipped scripts/propose.py, empty
-- rules); adopted versions start at 10. rules is a FULL snapshot (not a
-- delta). script_file NULL = run the shipped baseline script; otherwise a
-- file name under <app_local_data>/algorithms/ (resolution also checks
-- algorithms/archive/). Rows are inserted on explicit user adoption and
-- never UPDATEd; "last used" is derived from proposals.algorithm_version.

CREATE TABLE IF NOT EXISTS app_settings (
    key        VARCHAR PRIMARY KEY,
    value      VARCHAR NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS algorithm_versions (
    version       INTEGER PRIMARY KEY,
    description   VARCHAR NOT NULL,
    rules         JSON NOT NULL,
    script_file   VARCHAR,
    created_by    VARCHAR NOT NULL,      -- 'claude' | 'user'
    claude_run_id BIGINT,                -- provenance into claude_runs (app-enforced)
    adopted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
