# Sling-sourced roster + positions (empty start) — design

**Date:** 2026-06-06
**Status:** approved (design); implementation plan to follow
**Touches:** Sling integration (`sling-integration` skill), the schedule algorithm (`schedule-algorithm` skill — reproduce the prior month before publishing changes), a DuckDB migration (`schema-change` skill), the pull command, the Teachers page.

## Problem

Sling is the source of truth for who can teach, but the app doesn't reflect
that. It ships a placeholder roster (`Teacher A–J`, ids 1001–1010) and seeded
demo positions, and the pull only *updates* teachers already in the roster — new
Sling users are parked in a `sling_candidates` list for a manual "Add teacher"
click. The placeholder defaults obscure whether the app is actually wired to
Sling. The roster (and class list) should come directly from Sling, and the app
should start empty.

## Decision

Make the roster and positions Sling-sourced. A shared `sync_roster` routine
imports teachers + positions + qualifications from Sling; it runs on every month
pull AND from a new "Refresh from Sling" button on the Teachers page. The
placeholder seed is removed (empty start) and the manual add-teacher/candidates
flow retires. Departed/de-qualified teachers and removed positions are
**deactivated, never deleted** (schedule history references them). The proposer
never assigns an inactive teacher; an unfillable slot is left empty and flagged.

### Decisions locked during brainstorming
- **Import set (Q1):** Sling users who are active **and** members of the
  home-location group **and** qualified for ≥1 *active* (schedulable) position.
- **Removals (Q2):** deactivate (`active=false`) + hide from the working roster;
  never hard-delete (preserves FKs + months of history).
- **Algorithm (Q2 addendum):** the proposer draws only from active teachers; a
  historical slot whose teacher is now inactive is reassigned to an eligible
  active teacher, or left **unassigned and flagged** for manual correction —
  never force-assigned or silently dropped.
- **Positions (Q3 = B):** empty start; pull positions from Sling position-groups.
  `duration_minutes` defaults 60, `is_special` defaults false, `active` defaults
  true; the lead toggles `active` off for non-class groups (e.g. "Sales Rep").
  "Schedulable position" = `positions.active = true`.
- **New-teacher caps (Q4 = B):** newly imported teachers default to
  `weekly_target = 4`, `weekly_max = 5`, `variety_multiplier = 1.0`; the lead
  tunes from there. Preserved across re-syncs.
- **Trigger (Approach 3):** one `sync_roster` routine, called by the month pull
  and by a Teachers-page "Refresh from Sling" button.

## Why an explicit "schedulable" flag on positions

Sling position-type groups include non-class groups (the old recon excluded a
legacy "Teacher" group and "Sales Rep"; that hardcoded list was removed during
de-identification). Auto-importing all position-groups would create bogus class
types and make "qualified to teach" match non-teaching staff. `positions.active`
(already in the schema) is the lead-managed "this is a schedulable class type"
flag. Default true on import; the lead switches off non-class groups once
(preserved thereafter). The teacher import predicate keys off *active* positions.

## A. `sync_roster` routine

Lives in `commands.rs` (DB-heavy, transactional); pure decision helpers live in
`sling.rs` and are unit-tested. Inputs: the already-fetched `users` (Vec<SlingUser>)
and `groups` (Vec<SlingGroup>) plus `StudioConfig`. Runs inside one transaction.

Order (FK-safe):

1. **Positions** — for each Sling position-type group: upsert by id. New →
   `class_name = group.name, duration_minutes = 60, is_special = false,
   active = true`. Existing → update `class_name`; **preserve**
   `duration_minutes`, `is_special`, `active`. Any position whose id is not in
   the current Sling position-group set → `active = false` (never delete).
2. **Schedulable set** — `position_ids` where `positions.active = true`.
3. **Teachers** — import set = users where `active = true` AND the user's
   `group_ids` include the home-location group id AND `group_ids` intersect the
   schedulable position set. For each: new → insert with
   `weekly_target = 4, weekly_max = 5, variety_multiplier = 1.0,
   ranking_weight = 1.0, is_lead = (id == acting_user_id), active = true,
   display_name = name+lastname, locations = computed`. Existing → update
   `display_name`, `locations`, `active = true`, and set
   `is_lead = (id == acting_user_id)`; **preserve** `weekly_target`,
   `weekly_max`, `variety_multiplier`, `ranking_weight`, `notes`. Teachers in the
   table but not in the import set → `active = false` (never delete).
4. **Qualifications** — reconcile `teacher_qualifications` for each imported
   teacher against `group_ids ∩ schedulable position set`: insert missing,
   delete those no longer present (matches the pull's existing reconcile logic).

Returns a `RosterSyncSummary { teachers_active, teachers_deactivated,
positions_active, positions_deactivated }` for UI feedback.

Pure helpers in `sling.rs` (unit-tested):
- `fn is_schedulable_teacher(user: &SlingUser, home_location_id: i64, schedulable_position_ids: &HashSet<i64>) -> bool`
- `fn imported_user_ids(users: &[SlingUser], home_location_id, schedulable: &HashSet<i64>) -> HashSet<i64>` (or compute inline from the predicate)
- position-group extraction as `(id, name)` for the upsert.

## B. Empty start + cleanup

- `seed.rs`: stop seeding teachers, positions, and qualifications. A fresh
  install starts empty (the function becomes a no-op or is removed from the
  startup path). Document that the roster/positions arrive via a Sling refresh.
- **Migration `0008_drop_demo_roster.sql`** (forward-only, idempotent):
  - Delete `teacher_qualifications` rows for placeholder teacher ids 1001–1010.
  - Delete `teachers` rows with `sling_user_id BETWEEN 1001 AND 1010` **only when
    not referenced** by `proposal_shifts` (subquery guard) — so any with real
    history are left alone (they'd just be deactivated by the next sync).
  - `DROP TABLE IF EXISTS sling_candidates;` (the manual-candidate flow retires).
  - Idempotent: re-running deletes nothing further; `IF EXISTS` on the drop.
  - Seeded demo *positions* are NOT purged: on the real install their ids match
    real Sling position-groups (sync updates them); on a fresh install there are
    none. They need no migration.
- Retire: the `add_teacher_from_pull` and `list_sling_candidates` commands, the
  `sling_candidates` population in the pull, the `NewUserSummary` "new teacher
  detected" surfacing, and the corresponding Teachers-page UI.

## C. Algorithm behavior

`generate_proposal` already selects `FROM teachers WHERE active = TRUE`, so
inactive teachers never enter `propose.py`'s candidate pool — keep that.
`propose.py` builds the weekly slot template from the trailing-3-month shift
history. Requirement: a slot whose historical teacher is now inactive must be
assigned to an eligible **active** teacher by the normal ranking logic; if none
is eligible/available, the slot is emitted **unassigned** with a flag (e.g.
`flag = "needs_teacher"` / `generation_reason` noting the original teacher is
gone) rather than force-assigned or dropped. Surface these as an Issue in the
review/issues UI so the lead can assign manually. The exact `propose.py`
mechanism is confirmed/implemented under the `schedule-algorithm` skill, and the
previous month's output is reproduced before publishing the change.

## D. Frontend (Teachers page)

- **"Refresh from Sling"** button → new `refresh_roster_from_sling` command
  (fetches `users/concise` + `groups` with the saved token + studio config, runs
  `sync_roster`, returns the summary; shows e.g. "Synced N teachers, M positions
  from Sling"). Reuses 401 → token modal and unconfigured → clear message guards.
- Working roster lists **active** teachers only; inactive teachers are hidden
  (no separate UI this iteration).
- **Empty state**: when there are no teachers, show "No teachers yet — log in to
  Sling, set Studio configuration, then Refresh from Sling," rather than a blank
  table.
- Remove the add-teacher-from-pull form and the "new teacher detected" banner.
- Positions: the existing positions view (if any) lets the lead toggle a
  position's `active` (schedulable) flag; if no such control exists, add a simple
  active toggle so non-class groups can be excluded. (Scope: a minimal toggle, not
  a full positions manager.)

## E. Error handling & testing

- `refresh_roster_from_sling`: missing token → "log in to Sling first"; org/home
  not configured → the existing studio-config guard message; 401 → surfaces
  `sling-401` (handled like the pull). Sync runs in a single transaction —
  all-or-nothing.
- The month pull calls the same `sync_roster`; behavior is identical whether
  triggered by pull or button.
- **Rust unit tests**: `is_schedulable_teacher` (in/out by active, location,
  qualification); the deactivation-set computation (teacher present in table but
  not in import set → deactivated; present → active); position upsert
  preserve-vs-default (new gets defaults, existing keeps `is_special`/duration/
  active). 
- **Algorithm test**: a fixture where a historical slot's teacher is inactive →
  the generated proposal either reassigns to an active eligible teacher or emits
  the slot unassigned with the flag (not force-assigned).
- **Manual end-to-end**: fresh/empty app → log in → set studio config → Refresh
  from Sling → roster + positions populate from Sling; toggle a non-class position
  off → it leaves the qualified set; deactivate a teacher in Sling, re-refresh →
  they drop from the working roster but past proposals are intact; generate a
  month where a historical teacher is now inactive → slot reassigned or flagged.

## Out of scope

- A full positions manager (only a minimal `active` toggle if one doesn't exist).
- A "former teachers" UI (inactive teachers are simply hidden).
- Editing teacher identity/qualifications in-app (Sling owns those; the app owns
  only caps/variety/notes and the position `active`/`is_special`/`duration`
  metadata).
- Importing non-teaching staff or other locations.
