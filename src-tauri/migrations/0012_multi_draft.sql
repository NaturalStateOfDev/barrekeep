-- Migration 0012: several drafts per month, one push draft.
--
-- The lead teacher wants side-by-side drafts for the same month ("what if
-- teachers kept the same weekday+time all month?") and to toggle between
-- them, with Claude prompts applied to one or several drafts. A draft is just
-- a proposals row, so this adds metadata in SIDE TABLES instead of touching
-- `proposals` (which has incoming FKs from proposal_shifts and pushes — the
-- DuckDB UPDATE-as-DELETE+INSERT hazard, see CLAUDE.md / migrations 0003,
-- 0004, 0009). All three tables: PK only, no FKs in or out, nothing
-- references them.
--
-- proposal_drafts: name / parent / archived per proposal. Rename and archive
-- UPDATE non-indexed columns of a PK-only table with no incoming FKs — safe.
--
-- month_push_candidate: which draft Push sends for a month. Written with
-- INSERT OR REPLACE (app_settings pattern). This, not proposals.is_current,
-- is now the source of truth for "the" draft; is_current keeps meaning
-- "newest generated" for backward compatibility.
--
-- claude_run_targets: one Claude prompt applied to several drafts (one
-- claude_runs row per draft; claude_run_id = the prompt's first run).
--
-- Backfills are idempotent (INSERT ... WHERE NOT EXISTS) and numbering is
-- computed over ALL proposals before filtering, so a re-run never renames.

CREATE TABLE IF NOT EXISTS proposal_drafts (
    proposal_id        BIGINT PRIMARY KEY,
    name               VARCHAR NOT NULL,
    parent_proposal_id BIGINT,
    created_from       VARCHAR NOT NULL DEFAULT 'generate',
    archived           BOOLEAN NOT NULL DEFAULT FALSE,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS month_push_candidate (
    target_month VARCHAR PRIMARY KEY,
    proposal_id  BIGINT NOT NULL,
    set_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS claude_run_targets (
    claude_run_id BIGINT NOT NULL,
    proposal_id   BIGINT NOT NULL,
    PRIMARY KEY (claude_run_id, proposal_id)
);

-- "Draft 1", "Draft 2", ... per month in creation order.
INSERT INTO proposal_drafts (proposal_id, name, created_from, created_at)
SELECT n.id, 'Draft ' || CAST(n.rn AS VARCHAR), 'generate', n.generated_at
FROM (
    SELECT id, generated_at,
           row_number() OVER (PARTITION BY target_month ORDER BY generated_at, id) AS rn
    FROM proposals
) n
WHERE NOT EXISTS (SELECT 1 FROM proposal_drafts d WHERE d.proposal_id = n.id);

-- Push draft per month: the most recently pushed proposal if any was pushed
-- (that is what is in Sling), else the is_current one, else the newest.
INSERT INTO month_push_candidate (target_month, proposal_id)
SELECT r.target_month, r.id
FROM (
    SELECT p.target_month, p.id,
           row_number() OVER (
               PARTITION BY p.target_month
               ORDER BY lp.last_push_id DESC NULLS LAST,
                        p.is_current DESC, p.generated_at DESC, p.id DESC
           ) AS rn
    FROM proposals p
    LEFT JOIN (
        SELECT proposal_id, max(id) AS last_push_id FROM pushes GROUP BY proposal_id
    ) lp ON lp.proposal_id = p.id
) r
WHERE r.rn = 1
  AND NOT EXISTS (SELECT 1 FROM month_push_candidate m WHERE m.target_month = r.target_month);

INSERT INTO claude_run_targets (claude_run_id, proposal_id)
SELECT c.id, c.proposal_id
FROM claude_runs c
WHERE c.proposal_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM claude_run_targets t
      WHERE t.claude_run_id = c.id AND t.proposal_id = c.proposal_id
  );
