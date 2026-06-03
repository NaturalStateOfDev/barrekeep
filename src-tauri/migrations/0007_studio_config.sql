-- Migration 0007: runtime studio configuration (singleton).
-- Forward-only. Idempotent. Replaces the formerly compiled-in Sling org /
-- acting-user / home-location constants so the shipped binary carries no real
-- studio identity. Seeded with 0 placeholders; the app prompts the user to
-- fill these in Settings before the first pull.

CREATE TABLE IF NOT EXISTS studio_config (
    id               INTEGER PRIMARY KEY,         -- always 1 (singleton)
    org_id           BIGINT NOT NULL DEFAULT 0,
    acting_user_id   BIGINT NOT NULL DEFAULT 0,
    home_location_id BIGINT NOT NULL DEFAULT 0,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Ensure the singleton row exists exactly once.
INSERT INTO studio_config (id)
SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM studio_config WHERE id = 1);
