# Inactive teachers are excluded from the proposer; unfillable slots are flagged

**Date:** 2026-06-07
**Context:** Sling-sourced roster feature — departed/de-qualified teachers are
deactivated (`active = false`), never deleted (they're referenced by months of
proposal/shift history). The lead asked: when the algorithm builds a month from
the historical class grid, it must never place an inactive teacher; a slot whose
historical teacher is now gone should be filled by a current active teacher, or
left empty and flagged for manual correction — never force-assigned or silently
dropped.

## Finding: existing behavior already satisfies this — no algorithm change made

Verified the two relevant code paths:

1. **Candidate pool is active-only.** `generate_proposal` (src-tauri/src/commands.rs)
   selects the roster it passes to `propose.py` with `FROM teachers WHERE active = TRUE`.
   `propose.py` builds its `TEACHERS` / `TARGETS` / `RANKING_WEIGHTS` maps solely
   from that stdin `teachers` list. An inactive teacher is therefore absent from
   every candidate tier and can never be assigned.

2. **Unfillable slots are flagged, not dropped silently.** When the candidate
   waterfall (history tiers → class-type fallback → Sling-qualified → format-flex
   → lead overflow) can't fill a slot, `propose.py` appends it to `dropped_slots`
   and emits it into the proposal as `(…, teacher=None, reason="DROPPED",
   flag="DROP")` (propose.py ~line 592-593). The reviewer/issues UI surfaces these
   as flagged shifts for the lead to assign manually.

So a slot historically taught by a now-inactive teacher is automatically
reassigned to an eligible active teacher by the normal ranking logic, and only if
no active teacher is eligible/available does it land as a flagged DROP for manual
correction. This is exactly the required behavior; no code change was needed.

## Verification still owed (manual, with real data)

Per the schedule-algorithm skill, a live reproduction is part of the feature's
manual end-to-end validation: regenerate a month after marking a
historically-active teacher inactive, and confirm (a) zero assignments to that
teacher, (b) their former slots reassigned to active teachers or emitted as
flagged DROPs, (c) no disproportionate load shift elsewhere. Tracked in the
roster-from-sling plan's manual-validation section.

## Expected load impact

None for the common case (active roster unchanged). When a teacher goes inactive,
their historical slots redistribute to other active qualified teachers (raising
those teachers' load) or surface as flagged DROPs — both visible to the lead in
the review UI.
