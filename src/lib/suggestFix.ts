import type { ProposalShiftRow, Teacher, AvailabilityBlock } from "../types";
import { isoWeekKey } from "./dates";
import { slotAvailability, type AvailabilityLookup } from "./availability";

function weeklyCount(
  userId: number,
  isoWeek: string,
  shifts: ProposalShiftRow[],
): number {
  return shifts.filter(
    (s) => s.sling_user_id === userId && !s.is_dropped && isoWeekKey(s.shift_date) === isoWeek,
  ).length;
}

export function suggestSwap(
  target: ProposalShiftRow,
  allShifts: ProposalShiftRow[],
  teachers: Teacher[],
  qualifiedPairs: Set<string>,
  blocks: AvailabilityBlock[],
  /** Computed availability (null/omitted = not loaded; blocks decide). */
  availability?: AvailabilityLookup | null,
): Teacher | null {
  const week = isoWeekKey(target.shift_date);
  const candidates = teachers
    .filter((t) => t.active)
    .filter((t) => qualifiedPairs.has(`${t.sling_user_id}:${target.sling_position_id}`))
    // Only teachers whose windows contain the slot (unavailable, pending and
    // on-leave teachers are all out).
    .filter(
      (t) =>
        slotAvailability(blocks, availability, t.sling_user_id, target.shift_date, target.start_time, target.end_time).free,
    )
    .filter((t) => weeklyCount(t.sling_user_id, week, allShifts) < t.weekly_max);
  if (candidates.length === 0) return null;
  candidates.sort((a, b) => {
    if (b.ranking_weight !== a.ranking_weight) return b.ranking_weight - a.ranking_weight;
    const aw = weeklyCount(a.sling_user_id, week, allShifts);
    const bw = weeklyCount(b.sling_user_id, week, allShifts);
    if (aw !== bw) return aw - bw;
    return a.display_name.localeCompare(b.display_name);
  });
  return candidates[0];
}
