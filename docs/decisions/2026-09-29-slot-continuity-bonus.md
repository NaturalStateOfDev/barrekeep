# Slot continuity bonus (rule key `slot_continuity_bonus`)

**Date:** 2026-09-29
**Context:** Multi-draft feature. The lead wants to test "what if teachers were
on as consistent a day as possible — Jane always teaches Tuesdays at 8:45 and
9:45" by generating a second draft for the month and comparing it with the
first. Neither history ranking (tier 1 = who taught this slot in the trailing
3 months) nor the variety penalty (which actively rotates) expresses "keep
whoever got this slot in week 1".

## Rule

`slot_continuity_bonus` (number ≥ 0, default 0 = off), a rules-as-data key in
`algorithm_versions.rules` (validated in `src-tauri/src/algorithm.rs`,
applied in `scripts/propose.py`):

- propose.py tracks, per (weekday, start time), how many times each teacher
  has been assigned it so far this month (`slot_month_assignments`).
- When filling a slot with bonus > 0, teachers already holding that weekday +
  time join tier 1 (exact-slot) for it — moved out of the lower tiers so they
  aren't considered twice — and score `+ bonus × times_already_assigned`.
- Every hard filter (qualification, blocklists, availability, double booking,
  caps, special rules) still applies. No new tier is added.
- With the key absent or 0 the code path is skipped entirely and output is
  byte-identical to the baseline (`scripts/tests/test_propose_rules.py`
  cases 1 and 7). The output `parameters` only echo the key when > 0.

## Expected impact on load

Consistency trades against rotation: teachers who win a slot in week 1 keep
it, so load follows week 1's picks more closely and the variety penalty
(0.3/class by default) matters less. At 1.0 the bonus beats the penalty for
a teacher with ~3 more classes than a rival; at 3.0 it almost always wins.
Compare drafts (Compare tab: distinct weekday+time slots per teacher and the
share of classes in each teacher's most common slot) before adopting it into
the active algorithm version.

## Not done (yet)

"Optionally via history" — seeding the continuity count from the trailing
months — was left out: tier 1 already ranks by exact-slot history, so the
extra signal would double-count it.
