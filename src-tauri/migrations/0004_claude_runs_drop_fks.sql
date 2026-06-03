-- Migration 0004: rebuild claude_runs without FKs.
--
-- Same DuckDB UPDATE limitation as migration 0003: outgoing FKs interact
-- badly with UPDATE/DELETE on referenced tables. Also, prompt_id had a
-- NOT NULL FK to prompts(id), but we don't sync prompts to the DB yet —
-- the reviewer prompt currently lives inline in src-tauri/src/review.rs.
-- Until prompt syncing exists, prompt_id is nullable.
--
-- Application-level checks (proposal_id must exist when set, etc.) live
-- in src-tauri/src/commands.rs.

CREATE TABLE claude_runs_new (
    id            BIGINT PRIMARY KEY DEFAULT nextval('seq_claude_runs'),
    prompt_id     BIGINT,
    proposal_id   BIGINT,
    model         VARCHAR NOT NULL,
    input_tokens  INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    input_text    VARCHAR NOT NULL,
    output_text   VARCHAR NOT NULL,
    cost_usd      DECIMAL(10, 4) NOT NULL,
    duration_ms   INTEGER NOT NULL,
    ran_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO claude_runs_new
SELECT id, prompt_id, proposal_id, model, input_tokens, output_tokens,
       input_text, output_text, cost_usd, duration_ms, ran_at
FROM claude_runs;
DROP TABLE claude_runs;
ALTER TABLE claude_runs_new RENAME TO claude_runs;
