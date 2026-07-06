// Candidate list for the day-editor panel: every active teacher, annotated
// with why they might be a poor pick (not qualified / on leave / at cap).
// Unqualified teachers stay visible but disabled — positions are ground
// truth for "who can teach what" (see CLAUDE.md).

import type { ProposalShiftRow, Teacher, AvailabilityBlock } from "../types";
import { isoWeekKey } from "./dates";

export interface Candidate {
  teacher: Teacher;
  qualified: boolean;
  current: boolean;
  /** null when the teacher is a clean pick. */
  note: "not qualified" | "on leave" | "at weekly cap" | null;
}

function onLeave(blocks: AvailabilityBlock[], userId: number, startIso: string, endIso: string): boolean {
  return blocks.some(
    (b) => b.sling_user_id === userId && b.starts_at < endIso && b.ends_at > startIso,
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

      let note: Candidate["note"] = null;
      if (!qualified) note = "not qualified";
      else if (onLeave(blocks, t.sling_user_id, startIso, endIso)) note = "on leave";
      else if (weekly >= t.weekly_max) note = "at weekly cap";

      return { teacher: t, qualified, current, note };
    });

  // Clean qualified picks first (highest ranking weight, then name),
  // then flagged-but-qualified, then unqualified.
  const rank = (c: Candidate) => (!c.qualified ? 2 : c.note ? 1 : 0);
  out.sort((a, b) => {
    if (rank(a) !== rank(b)) return rank(a) - rank(b);
    if (b.teacher.ranking_weight !== a.teacher.ranking_weight) {
      return b.teacher.ranking_weight - a.teacher.ranking_weight;
    }
    return a.teacher.display_name.localeCompare(b.teacher.display_name);
  });
  return out;
}
