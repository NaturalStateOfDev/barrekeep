-- Migration 0002: add coteach_label to proposal_shifts.
-- propose.py emits co-teach as a label string ("Teacher A + Teacher E")
-- attached to a single row, rather than two rows linked via
-- coteach_partner_shift_id. We preserve that representation here.
-- Eventually the Sling push will need to expand co-teach into two records;
-- when we get there, this column becomes the source for the second row.

ALTER TABLE proposal_shifts ADD COLUMN IF NOT EXISTS coteach_label VARCHAR;
