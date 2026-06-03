-- Migration 0001: core schema
-- Mirrors the DDL in docs/data-model.md. Update both together.

-- ---------- sequences ----------
CREATE SEQUENCE IF NOT EXISTS seq_availability;
CREATE SEQUENCE IF NOT EXISTS seq_proposals;
CREATE SEQUENCE IF NOT EXISTS seq_proposal_shifts;
CREATE SEQUENCE IF NOT EXISTS seq_edits;
CREATE SEQUENCE IF NOT EXISTS seq_prompts;
CREATE SEQUENCE IF NOT EXISTS seq_claude_runs;
CREATE SEQUENCE IF NOT EXISTS seq_pushes;
CREATE SEQUENCE IF NOT EXISTS seq_push_results;

-- ---------- core reference tables ----------
CREATE TABLE IF NOT EXISTS teachers (
    sling_user_id      INTEGER PRIMARY KEY,
    display_name       VARCHAR NOT NULL,
    weekly_target      INTEGER NOT NULL,
    weekly_max         INTEGER NOT NULL,
    is_lead            BOOLEAN NOT NULL DEFAULT FALSE,
    ranking_weight     DOUBLE  NOT NULL DEFAULT 1.0,
    variety_multiplier DOUBLE  NOT NULL DEFAULT 1.0,
    active             BOOLEAN NOT NULL DEFAULT TRUE,
    notes              VARCHAR,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS positions (
    sling_position_id  INTEGER PRIMARY KEY,
    class_name         VARCHAR NOT NULL UNIQUE,
    duration_minutes   INTEGER NOT NULL DEFAULT 60,
    is_special         BOOLEAN NOT NULL DEFAULT FALSE,
    active             BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE IF NOT EXISTS teacher_qualifications (
    sling_user_id      INTEGER NOT NULL REFERENCES teachers(sling_user_id),
    sling_position_id  INTEGER NOT NULL REFERENCES positions(sling_position_id),
    is_blocklisted     BOOLEAN NOT NULL DEFAULT FALSE,
    blocklist_reason   VARCHAR,
    PRIMARY KEY (sling_user_id, sling_position_id)
);

-- ---------- pulled data ----------
CREATE TABLE IF NOT EXISTS availability_blocks (
    id            BIGINT PRIMARY KEY DEFAULT nextval('seq_availability'),
    sling_user_id INTEGER NOT NULL REFERENCES teachers(sling_user_id),
    source        VARCHAR NOT NULL,
    starts_at     TIMESTAMPTZ NOT NULL,
    ends_at       TIMESTAMPTZ NOT NULL,
    pulled_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_avail_user_time
    ON availability_blocks(sling_user_id, starts_at, ends_at);

-- ---------- proposals ----------
CREATE TABLE IF NOT EXISTS proposals (
    id                BIGINT PRIMARY KEY DEFAULT nextval('seq_proposals'),
    target_month      VARCHAR NOT NULL,
    algorithm_version VARCHAR NOT NULL,
    parameters        JSON NOT NULL,
    generated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    notes             VARCHAR,
    is_current        BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS proposal_shifts (
    id                       BIGINT PRIMARY KEY DEFAULT nextval('seq_proposal_shifts'),
    proposal_id              BIGINT NOT NULL REFERENCES proposals(id),
    shift_date               DATE NOT NULL,
    start_time               VARCHAR NOT NULL,
    end_time                 VARCHAR NOT NULL,
    sling_position_id        INTEGER NOT NULL REFERENCES positions(sling_position_id),
    sling_user_id            INTEGER REFERENCES teachers(sling_user_id),
    generation_reason        VARCHAR NOT NULL,
    flag                     VARCHAR,
    is_coteach               BOOLEAN NOT NULL DEFAULT FALSE,
    coteach_partner_shift_id BIGINT REFERENCES proposal_shifts(id),
    is_dropped               BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX IF NOT EXISTS idx_prop_shifts_date
    ON proposal_shifts(proposal_id, shift_date, start_time);

CREATE TABLE IF NOT EXISTS edits (
    id                BIGINT PRIMARY KEY DEFAULT nextval('seq_edits'),
    proposal_shift_id BIGINT NOT NULL REFERENCES proposal_shifts(id),
    field             VARCHAR NOT NULL,
    old_value         VARCHAR,
    new_value         VARCHAR,
    reason            VARCHAR,
    edited_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    reverted          BOOLEAN NOT NULL DEFAULT FALSE
);

-- ---------- prompts + Claude runs ----------
CREATE TABLE IF NOT EXISTS prompts (
    id          BIGINT PRIMARY KEY DEFAULT nextval('seq_prompts'),
    name        VARCHAR NOT NULL,
    version     INTEGER NOT NULL,
    body        VARCHAR NOT NULL,
    source_file VARCHAR,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (name, version)
);

CREATE TABLE IF NOT EXISTS claude_runs (
    id            BIGINT PRIMARY KEY DEFAULT nextval('seq_claude_runs'),
    prompt_id     BIGINT NOT NULL REFERENCES prompts(id),
    proposal_id   BIGINT REFERENCES proposals(id),
    model         VARCHAR NOT NULL,
    input_tokens  INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    input_text    VARCHAR NOT NULL,
    output_text   VARCHAR NOT NULL,
    cost_usd      DECIMAL(10, 4) NOT NULL,
    duration_ms   INTEGER NOT NULL,
    ran_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------- pushes ----------
CREATE TABLE IF NOT EXISTS pushes (
    id               BIGINT PRIMARY KEY DEFAULT nextval('seq_pushes'),
    proposal_id      BIGINT NOT NULL REFERENCES proposals(id),
    started_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at      TIMESTAMPTZ,
    shifts_attempted INTEGER NOT NULL DEFAULT 0,
    shifts_succeeded INTEGER NOT NULL DEFAULT 0,
    shifts_failed    INTEGER NOT NULL DEFAULT 0,
    shifts_skipped   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS push_results (
    id                BIGINT PRIMARY KEY DEFAULT nextval('seq_push_results'),
    push_id           BIGINT NOT NULL REFERENCES pushes(id),
    proposal_shift_id BIGINT NOT NULL REFERENCES proposal_shifts(id),
    outcome           VARCHAR NOT NULL,
    sling_shift_id    VARCHAR,
    error_message     VARCHAR,
    attempted_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    retry_count       INTEGER NOT NULL DEFAULT 0
);
