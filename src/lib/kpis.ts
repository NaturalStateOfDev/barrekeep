// KPI header numbers for a proposal (coverage / balance / hours).
// Open-conflicts comes from computeIssues() and is passed straight through
// by the screen, so it isn't computed here.

import type { ProposalShiftRow } from "../types";

export interface Kpis {
  totalCount: number; // non-dropped shifts
  assignedCount: number; // non-dropped with a teacher (incl. co-teach rows)
  coveragePct: number; // assigned / total, rounded; 0 when empty
  teacherHours: number; // scheduled hours across assigned non-dropped shifts, rounded
  teacherCount: number; // distinct assigned teachers
  balance: "Even" | "Uneven" | "—";
}

function hoursBetween(start: string, end: string): number {
  const [sh, sm] = start.split(":").map(Number);
  const [eh, em] = end.split(":").map(Number);
  return (eh * 60 + em - (sh * 60 + sm)) / 60;
}

function isAssigned(s: ProposalShiftRow): boolean {
  return s.sling_user_id != null || s.coteach_label != null;
}

export function computeKpis(shifts: ProposalShiftRow[]): Kpis {
  const live = shifts.filter((s) => !s.is_dropped);
  const assigned = live.filter(isAssigned);

  const hours = assigned.reduce((sum, s) => sum + hoursBetween(s.start_time, s.end_time), 0);

  const perTeacher = new Map<number, number>();
  for (const s of assigned) {
    if (s.sling_user_id == null) continue;
    perTeacher.set(s.sling_user_id, (perTeacher.get(s.sling_user_id) ?? 0) + 1);
  }

  // Balance: coefficient of variation of per-teacher class counts. Not
  // target-aware (targets are weekly and weeks vary per month); a rough
  // spread signal is what the KPI card needs.
  let balance: Kpis["balance"] = "—";
  if (perTeacher.size >= 2) {
    const counts = [...perTeacher.values()];
    const mean = counts.reduce((a, b) => a + b, 0) / counts.length;
    const variance = counts.reduce((a, b) => a + (b - mean) ** 2, 0) / counts.length;
    const cv = Math.sqrt(variance) / mean;
    balance = cv <= 0.4 ? "Even" : "Uneven";
  }

  return {
    totalCount: live.length,
    assignedCount: assigned.length,
    coveragePct: live.length === 0 ? 0 : Math.round((assigned.length / live.length) * 100),
    teacherHours: Math.round(hours),
    teacherCount: perTeacher.size,
    balance,
  };
}
