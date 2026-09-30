// Pure helpers for multiple drafts per month (see src-tauri/src/drafts.rs).

import type { DraftSlotDiff, ProposalShiftRow, ProposalSummary, TeacherStats } from "../types";

export const MAX_DRAFT_NAME = 60;

/** Drafts of one month, newest first (the list arrives id DESC). Archived
 *  drafts are hidden unless asked for — except the one being viewed. */
export function draftsForMonth(
  proposals: ProposalSummary[],
  month: string,
  showArchived: boolean,
  viewingId?: number | null,
): ProposalSummary[] {
  return proposals.filter(
    (p) => p.target_month === month && (showArchived || !p.archived || p.id === viewingId),
  );
}

/** The month's push draft, if any. */
export function pushDraftFor(
  proposals: ProposalSummary[],
  month: string,
): ProposalSummary | undefined {
  return proposals.find((p) => p.target_month === month && p.is_push_candidate);
}

/** The draft to open when a month is picked: its push draft, else the newest
 *  non-archived one, else the newest. */
export function representativeDraft(
  proposals: ProposalSummary[],
  month: string,
): ProposalSummary | undefined {
  const inMonth = proposals.filter((p) => p.target_month === month);
  return (
    inMonth.find((p) => p.is_push_candidate) ??
    inMonth.find((p) => !p.archived) ??
    inMonth[0]
  );
}

/** Default name for a what-if copy. */
export function whatIfName(name: string): string {
  const suffix = " · what-if";
  const room = MAX_DRAFT_NAME - suffix.length;
  const base = name.length > room ? name.slice(0, room - 1) + "…" : name;
  return base + suffix;
}

/** Default Compare pair: push draft vs the draft being viewed (or, when
 *  viewing the push draft, the newest other draft). */
export function defaultComparePair(
  drafts: ProposalSummary[],
  viewingId: number,
): [number, number] | null {
  if (drafts.length < 2) return null;
  const push = drafts.find((d) => d.is_push_candidate) ?? drafts[drafts.length - 1];
  if (viewingId !== push.id && drafts.some((d) => d.id === viewingId)) return [push.id, viewingId];
  const other = drafts.find((d) => d.id !== push.id)!;
  return [push.id, other.id];
}

export interface DiffFilter {
  /** Teacher display name ("" = any). Matches either side, incl. co-teach labels. */
  teacher: string;
  /** "Mon".."Sun" ("" = any). */
  weekday: string;
}

export function filterDiff(changes: DraftSlotDiff[], f: DiffFilter): DraftSlotDiff[] {
  return changes.filter((c) => {
    if (f.weekday && c.weekday !== f.weekday) return false;
    if (f.teacher) {
      const hit = [c.teacher_a, c.teacher_b].some((t) => t != null && t.includes(f.teacher));
      if (!hit) return false;
    }
    return true;
  });
}

export const DIFF_KIND_LABEL: Record<string, string> = {
  teacher: "teacher",
  format: "format",
  format_teacher: "format + teacher",
  only_a: "only in A",
  only_b: "only in B",
};

/** Cost line for a prompt sent to `n` drafts: one Claude call per draft. */
export function estimateCostLabel(lastCostPerDraft: number | null, n: number): string {
  if (n <= 0) return "Pick at least one draft.";
  const calls = `${n} Claude call${n === 1 ? "" : "s"} (one per draft)`;
  if (lastCostPerDraft == null) return n === 1 ? calls : `${calls} — about ${n}× the cost of one request`;
  return `${calls} — est. $${(lastCostPerDraft * n).toFixed(2)}`;
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEK_ORDER = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

function weekdayOf(isoDate: string): string {
  return WEEKDAYS[new Date(isoDate + "T12:00:00Z").getUTCDay()];
}

function slotOrder(label: string): [number, string] {
  const [wd, time = ""] = label.split(" ");
  const i = WEEK_ORDER.indexOf(wd);
  return [i < 0 ? 7 : i, time];
}

/** Per-teacher consistency (mirrors drafts::consistency_stats in Rust; used
 *  by the browser dev mock). Counts assigned, non-dropped rows by
 *  sling_user_id. */
export function consistencyStats(shifts: ProposalShiftRow[]): Map<number, TeacherStats> {
  const per = new Map<number, Map<string, number>>();
  for (const s of shifts) {
    if (s.sling_user_id == null || s.is_dropped) continue;
    const slot = `${weekdayOf(s.shift_date)} ${s.start_time}`;
    const m = per.get(s.sling_user_id) ?? new Map<string, number>();
    m.set(slot, (m.get(slot) ?? 0) + 1);
    per.set(s.sling_user_id, m);
  }
  const out = new Map<number, TeacherStats>();
  for (const [uid, slots] of per) {
    let classes = 0;
    let top = "";
    let topCount = 0;
    for (const [slot, n] of slots) {
      classes += n;
      const better =
        n > topCount ||
        (n === topCount &&
          (() => {
            const [a, at] = slotOrder(slot);
            const [b, bt] = slotOrder(top);
            return a < b || (a === b && at < bt);
          })());
      if (better) {
        top = slot;
        topCount = n;
      }
    }
    out.set(uid, {
      classes,
      distinct_slots: slots.size,
      top_slot: top,
      top_slot_count: topCount,
      top_slot_share: classes > 0 ? topCount / classes : 0,
    });
  }
  return out;
}

export function pct(share: number): string {
  return `${Math.round(share * 100)}%`;
}
