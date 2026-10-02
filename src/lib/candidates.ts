// Candidate list for the day-editor panel: the whole active roster, each
// teacher marked trained (qualified per Sling positions — ground truth, see
// CLAUDE.md) and available (the slot sits inside one of their available
// windows — i.e. no unavailability or leave over it — and they are under
// their weekly cap). Untrained teachers stay visible but unselectable.

import type { ProposalShiftRow, Teacher, AvailabilityBlock } from "../types";
import { isoWeekKey } from "./dates";
import { blockTag, slotAvailability, type AvailabilityLookup, type BlockKind } from "./availability";

export interface Candidate {
  teacher: Teacher;
  /** Qualified for this class per Sling positions ("trained"). */
  qualified: boolean;
  /** A leave block covers the slot. */
  on_leave: boolean;
  /** Unavailable for the slot for a reason other than leave: a (possibly
   *  pending) unavailability block, or simply outside their windows. */
  unavailable: boolean;
  /** Why the slot is blocked, if it is: "leave" | "unavailable" | "pending". */
  blocked_by: BlockKind | null;
  at_cap: boolean;
  /** Free for the slot (not on leave, not unavailable) and under weekly cap. */
  available: boolean;
  current: boolean;
}

/** The availability chip for a candidate: text + tone. */
export function availabilityTag(c: Candidate): { text: string; tone: "ok" | "warn" | "danger" } {
  if (c.blocked_by) return { text: blockTag(c.blocked_by), tone: "danger" };
  if (c.at_cap) return { text: "at cap", tone: "warn" };
  return { text: "available", tone: "ok" };
}

export function candidatesFor(
  target: ProposalShiftRow,
  allShifts: ProposalShiftRow[],
  teachers: Teacher[],
  qualifiedPairs: Set<string>,
  blocks: AvailabilityBlock[],
  /** Computed availability (null/omitted = not loaded; blocks decide). */
  availability?: AvailabilityLookup | null,
): Candidate[] {
  const week = isoWeekKey(target.shift_date);

  const out: Candidate[] = teachers
    .filter((t) => t.active)
    .map((t) => {
      const qualified = qualifiedPairs.has(`${t.sling_user_id}:${target.sling_position_id}`);
      const current = target.sling_user_id === t.sling_user_id;
      // Count the teacher's classes that week, excluding the target slot
      // itself — reassigning them to their own class shouldn't read as
      // pushing them over cap.
      const weekly = allShifts.filter(
        (s) =>
          s.id !== target.id &&
          !s.is_dropped &&
          s.sling_user_id === t.sling_user_id &&
          isoWeekKey(s.shift_date) === week,
      ).length;

      const { free, reason } = slotAvailability(
        blocks, availability, t.sling_user_id, target.shift_date, target.start_time, target.end_time,
      );
      const at_cap = weekly >= t.weekly_max;

      return {
        teacher: t,
        qualified,
        on_leave: reason === "leave",
        unavailable: !free && reason !== "leave",
        blocked_by: reason,
        at_cap,
        available: free && !at_cap,
        current,
      };
    });

  // Trained first, then available, then alphabetical.
  out.sort((a, b) => {
    if (a.qualified !== b.qualified) return a.qualified ? -1 : 1;
    if (a.available !== b.available) return a.available ? -1 : 1;
    return a.teacher.display_name.localeCompare(b.teacher.display_name);
  });
  return out;
}
