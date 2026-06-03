import type { ProposalShiftRow, Teacher, AvailabilityBlock } from "../types";
import { isoWeekKey } from "./dates";

export type IssueKind =
  | "unassigned"
  | "over_cap"
  | "qualification"
  | "leave_conflict"
  | "teacher_deactivated"
  | "external_shift"
  | "new_teacher";

export interface Issue {
  kind: IssueKind;
  shift_id: number | null;
  shift_date: string | null;
  message: string;
  /** Free-form per-kind extra payload (e.g., external shift's sling_shift_id or new user id). */
  ref?: number | string;
}

interface ExternalShiftInput {
  sling_shift_id: number;
  shift_date: string;
  start_time: string;
  sling_user_id: number | null;
  sling_position_id: number;
}

interface NewUserInput {
  sling_user_id: number;
  display_name: string;
  locations?: string | null;
}

export function computeIssues(
  shifts: ProposalShiftRow[],
  teachers: Teacher[],
  qualifiedPairs: Set<string>,
  blocks: AvailabilityBlock[],
  externalShifts: ExternalShiftInput[],
  newUsers: NewUserInput[],
): Issue[] {
  const out: Issue[] = [];
  const teacherById = new Map(teachers.map((t) => [t.sling_user_id, t]));

  // Unassigned
  for (const s of shifts) {
    if (s.is_dropped) continue;
    if (s.sling_user_id == null) {
      out.push({
        kind: "unassigned",
        shift_id: s.id,
        shift_date: s.shift_date,
        message: `${s.start_time} ${s.class_name} unassigned`,
      });
    }
  }

  // Over cap
  const counts = new Map<string, { teacher: Teacher; shift: ProposalShiftRow; count: number }>();
  for (const s of shifts) {
    if (s.is_dropped || s.sling_user_id == null) continue;
    const t = teacherById.get(s.sling_user_id);
    if (!t) continue;
    const key = `${s.sling_user_id}:${isoWeekKey(s.shift_date)}`;
    const prev = counts.get(key);
    if (prev) {
      prev.count += 1;
      // Anchor the issue on the latest assignment in this week so Apply
      // swaps a shift that actually pushed the teacher over (not the first).
      const prevKey = `${prev.shift.shift_date}T${prev.shift.start_time}`;
      const curKey = `${s.shift_date}T${s.start_time}`;
      if (curKey > prevKey) prev.shift = s;
    }
    else counts.set(key, { teacher: t, shift: s, count: 1 });
  }
  for (const { teacher, shift, count } of counts.values()) {
    if (count > teacher.weekly_max) {
      out.push({
        kind: "over_cap",
        shift_id: shift.id,
        shift_date: shift.shift_date,
        message: `${teacher.display_name} over weekly cap (${count} / ${teacher.weekly_max})`,
      });
    }
  }

  // Qualification mismatch
  for (const s of shifts) {
    if (s.is_dropped || s.sling_user_id == null) continue;
    const pair = `${s.sling_user_id}:${s.sling_position_id}`;
    if (!qualifiedPairs.has(pair)) {
      const t = teacherById.get(s.sling_user_id);
      out.push({
        kind: "qualification",
        shift_id: s.id,
        shift_date: s.shift_date,
        message: `${t?.display_name ?? "?"} not qualified for ${s.class_name}`,
      });
    }
  }

  // Leave conflict
  for (const s of shifts) {
    if (s.is_dropped || s.sling_user_id == null) continue;
    const tEnd = `${s.shift_date}T${s.end_time}:00`;
    const tStart = `${s.shift_date}T${s.start_time}:00`;
    const conflicts = blocks.some(
      (b) => b.sling_user_id === s.sling_user_id && b.starts_at < tEnd && b.ends_at > tStart,
    );
    if (conflicts) {
      const t = teacherById.get(s.sling_user_id);
      out.push({
        kind: "leave_conflict",
        shift_id: s.id,
        shift_date: s.shift_date,
        message: `${t?.display_name ?? "?"} has leave overlapping ${s.start_time} ${s.class_name}`,
      });
    }
  }

  // Teacher deactivated
  for (const s of shifts) {
    if (s.is_dropped || s.sling_user_id == null) continue;
    const t = teacherById.get(s.sling_user_id);
    if (t && !t.active) {
      out.push({
        kind: "teacher_deactivated",
        shift_id: s.id,
        shift_date: s.shift_date,
        message: `${t.display_name} is deactivated in Sling`,
      });
    }
  }

  // External shift not in proposal
  const proposalFingerprints = new Set(
    shifts.filter((s) => !s.is_dropped).map(
      (s) => `${s.shift_date}|${s.start_time}|${s.sling_position_id}`,
    ),
  );
  for (const ext of externalShifts) {
    const fp = `${ext.shift_date}|${ext.start_time}|${ext.sling_position_id}`;
    if (!proposalFingerprints.has(fp)) {
      out.push({
        kind: "external_shift",
        shift_id: null,
        shift_date: ext.shift_date,
        message: `Sling shift ${ext.shift_date} ${ext.start_time} (pos ${ext.sling_position_id}) not in proposal`,
        ref: ext.sling_shift_id,
      });
    }
  }

  // New teacher in Sling
  for (const u of newUsers) {
    const locSuffix = u.locations ? ` — ${u.locations}` : "";
    out.push({
      kind: "new_teacher",
      shift_id: null,
      shift_date: null,
      message: `New in Sling: ${u.display_name}${locSuffix}`,
      ref: u.sling_user_id,
    });
  }

  return out;
}
