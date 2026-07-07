// Candidate list for the day-editor panel: the whole active roster, each
// teacher marked trained (qualified per Sling positions — ground truth, see
// CLAUDE.md) and available (no leave block over the slot, under weekly cap).
// Untrained teachers stay visible but unselectable.

import type { ProposalShiftRow, Teacher, AvailabilityBlock } from "../types";
import { isoWeekKey, wallClock } from "./dates";

export interface Candidate {
  teacher: Teacher;
  /** Qualified for this class per Sling positions ("trained"). */
  qualified: boolean;
  on_leave: boolean;
  at_cap: boolean;
  /** No leave conflict and under weekly cap. */
  available: boolean;
  current: boolean;
}

function onLeave(blocks: AvailabilityBlock[], userId: number, startIso: string, endIso: string): boolean {
  return blocks.some(
    (b) =>
      b.sling_user_id === userId &&
      wallClock(b.starts_at) < endIso &&
      wallClock(b.ends_at) > startIso,
  );
}

export function candidatesFor(
  target: ProposalShiftRow,
  allShifts: ProposalShiftRow[],
  teachers: Teacher[],
  qualifiedPairs: Set<string>,
  blocks: AvailabilityBlock[],
): Candidate[] {
  const week = isoWeekKey(target.shift_date);
  const startIso = `${target.shift_date}T${target.start_time}:00`;
  const endIso = `${target.shift_date}T${target.end_time}:00`;

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

      const on_leave = onLeave(blocks, t.sling_user_id, startIso, endIso);
      const at_cap = weekly >= t.weekly_max;

      return {
        teacher: t,
        qualified,
        on_leave,
        at_cap,
        available: !on_leave && !at_cap,
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
