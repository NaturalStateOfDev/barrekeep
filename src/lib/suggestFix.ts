import type { ProposalShiftRow, Teacher, AvailabilityBlock } from "../types";
import { isoWeekKey, wallClock } from "./dates";

function overlaps(
  blocks: AvailabilityBlock[],
  userId: number,
  shiftStartIso: string,
  shiftEndIso: string,
): boolean {
  return blocks.some((b) => {
    if (b.sling_user_id !== userId) return false;
    return wallClock(b.starts_at) < shiftEndIso && wallClock(b.ends_at) > shiftStartIso;
  });
}

function weeklyCount(
  userId: number,
  isoWeek: string,
  shifts: ProposalShiftRow[],
): number {
  return shifts.filter(
    (s) => s.sling_user_id === userId && !s.is_dropped && isoWeekKey(s.shift_date) === isoWeek,
  ).length;
}

function shiftStartEndIso(s: ProposalShiftRow): [string, string] {
  return [
    `${s.shift_date}T${s.start_time}:00`,
    `${s.shift_date}T${s.end_time}:00`,
  ];
}

export function suggestSwap(
  target: ProposalShiftRow,
  allShifts: ProposalShiftRow[],
  teachers: Teacher[],
  qualifiedPairs: Set<string>,
  blocks: AvailabilityBlock[],
): Teacher | null {
  const week = isoWeekKey(target.shift_date);
  const [tStart, tEnd] = shiftStartEndIso(target);
  const candidates = teachers
    .filter((t) => t.active)
    .filter((t) => qualifiedPairs.has(`${t.sling_user_id}:${target.sling_position_id}`))
    .filter((t) => !overlaps(blocks, t.sling_user_id, tStart, tEnd))
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
