---
name: schedule-algorithm
description: Use this skill when modifying or extending the schedule-generation algorithm — adding rules, changing tier ordering, adjusting variety penalties, handling new edge cases. Algorithm changes affect every future month and are hard to reason about in isolation. Always reproduce the previous month's output before publishing changes.
---

# Working on the schedule-generation algorithm

The algorithm is the heart of this app. Every change to it affects the next month's schedule and may have cascading effects months later. Be deliberate.

## Algorithm structure (v10 baseline)

The algorithm runs in this order:

1. **Build class grid** from prior month's recurring slots (weekday × time → class type, with biweekly detection).
2. **Place Focus classes** manually (3 placements: weekend, mid-morning, evening).
3. **Place hard-assignment overrides** (manager-specified date+time+teacher).
4. **For each remaining slot**, run the candidate-selection waterfall:
   - Tier 1: April-history primary candidates (taught this exact slot)
   - Tier 2: Same class+day fallback (taught this class on this weekday)
   - Tier 3: Class type fallback (taught this class anywhere)
   - Tier 4: Sling-qualified, no April history (positioned for it but never assigned)
5. **Within each tier**, rank by `experience × ranking_weight - variety_penalty × monthly_load × per_teacher_multiplier`.
6. **Two passes per tier**: first under-target candidates, then under-max.
7. **Hard rule filters**: not blocked, not double-booked, not slot-blocked, not date-blocked, passes special rules.
8. **Format-flex fallback**: try alternate class types if primary class can't be filled.
9. **Teacher A overflow**: last-resort assignment to Teacher A (lead teacher).
10. **Drop the class** if even Teacher A can't cover it.

## Always do these

1. **Test against the previous month before changing anything.** Run the algorithm with the previous month's data and confirm the output matches the published schedule (within a tolerance — exact match isn't required, but big load shifts are red flags).

2. **Add new constraints as data, not code.** New special rules go into a config table or YAML, not as inline `if uid == 12345` checks. Code that assumes specific user IDs is fragile.

3. **Document the rule in `docs/decisions/`** when adding a new constraint. Include: the manager's rationale, the date introduced, and the expected impact on load.

4. **Keep the per-teacher rules transparent.** Every special rule should be visible in the UI when reviewing the schedule (a tooltip explaining why a teacher was/wasn't a candidate).

## Never do these

- **Never silently drop slots.** Every dropped class must be flagged with a clear reason (e.g., "no qualified teacher available") and surfaced in the UI for manual override.

- **Never hardcode month numbers.** The algorithm should work for any target month. June-specific overrides go into config keyed by year-month.

- **Never let the variety penalty override hard rules.** A teacher who is blocked, on leave, or unqualified must NEVER be selected, no matter how low their monthly load is.

- **Never add more than 3 retry tiers.** If the first 4 tiers + format-flex + Teacher A overflow can't fill a slot, the right answer is to drop it and let the manager decide. Adding tier 5, 6, 7 makes the assignment reasoning impossible to debug.

## When tuning the variety penalty

The default is `VARIETY_PENALTY_PER_CLASS = 0.3`. Rules of thumb:

- **0.0 (off):** April-history teachers get every slot; new hires never get a turn.
- **0.15:** mild rotation; primary teachers still dominate but new hires get filler slots.
- **0.3 (current):** balanced; primaries get most slots, others get noticeable variety.
- **0.5:** aggressive rotation; primaries lose to less-loaded teachers if any are qualified.
- **1.0+:** chaotic; the algorithm essentially load-balances by recent count.

The per-teacher multiplier amplifies. Teacher G has `3.0` because they're at a 2/wk cap and we want to avoid pushing them over. To boost a teacher (give them more), use a multiplier `< 1.0`.

## Comparing two algorithm versions

Always produce a side-by-side diff:

```
WD/Time          v9 teacher        v10 teacher       Change
Mon 5:45 Define  Teacher E     Teacher E     same
Tue 5:45 Reform  Teacher D    Teacher A     SWAP
...
```

And a load comparison:

```
Teacher              v9    v10   Δ
Teacher A        14    20    +6
Teacher B       23    19    -4
...
```

These two views catch the most common regression: a single rule change cascades into a disproportionate load shift.

## Reference: hard-rule cheat sheet

| Constraint | Source | How to test |
|---|---|---|
| Teacher qualified for class | Sling group memberships | `teacher_qualifications` table |
| Teacher available at time | Pulled from Sling | `availability_blocks` table |
| Teacher not double-booked | This run's assignments | In-memory tracking |
| Manager class blocklist | `teacher_class_blocklist` | e.g., Teacher E × Reform |
| Teacher slot blocklist | `teacher_slot_blocklist` | e.g., Teacher B × all 5:45am |
| Teacher date blocklist | `teacher_date_blocklist` | e.g., Teacher J × Wed Jun 17 5:45am |
| Special rule (Teacher I caps) | Custom function | Test with synthetic week |
