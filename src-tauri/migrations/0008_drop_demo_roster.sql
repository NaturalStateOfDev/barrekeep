-- Migration 0008: remove the placeholder demo roster + retire the manual
-- add-teacher candidates flow. The roster is now synced from Sling.
--
-- Placeholder teachers were seeded with ids 1001..1010 ("Teacher A".."J").
-- Delete them (and their qualifications) ONLY when no proposal references them,
-- so any with real history are preserved (the Sling sync will deactivate those
-- instead). Idempotent: re-running deletes nothing further.

DELETE FROM teacher_qualifications
WHERE sling_user_id BETWEEN 1001 AND 1010
  AND sling_user_id NOT IN (SELECT sling_user_id FROM proposal_shifts WHERE sling_user_id IS NOT NULL);

-- availability_blocks also has a NOT NULL FK to teachers(sling_user_id); clear
-- any placeholder rows first or the teachers delete below would abort on the FK.
DELETE FROM availability_blocks
WHERE sling_user_id BETWEEN 1001 AND 1010
  AND sling_user_id NOT IN (SELECT sling_user_id FROM proposal_shifts WHERE sling_user_id IS NOT NULL);

DELETE FROM teachers
WHERE sling_user_id BETWEEN 1001 AND 1010
  AND sling_user_id NOT IN (SELECT sling_user_id FROM proposal_shifts WHERE sling_user_id IS NOT NULL);

-- The manual "add teacher from pull" candidates table is no longer used.
DROP TABLE IF EXISTS sling_candidates;
