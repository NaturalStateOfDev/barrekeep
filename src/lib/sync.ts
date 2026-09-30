// Helpers for the push-sync preview and the availability-refresh conflict
// list (see src-tauri/src/push_sync.rs and conflicts.rs).

import type { DraftConflictKind, ProposalSummary, ShiftView, SyncAction, SyncActionKind, SyncPreview } from "../types";
import { formatDayShort, formatTime, formatTimeRange, type TimeFormat } from "./dates";

export interface SyncGroups {
  create: SyncAction[];
  update: SyncAction[];
  delete: SyncAction[];
  cleanup: SyncAction[];
  adopt: SyncAction[];
  baseline: SyncAction[];
  /** Skips because the shift was deleted in Sling since the last push. */
  deletedInSling: SyncAction[];
  skip: SyncAction[];
}

export function groupSyncActions(actions: SyncAction[]): SyncGroups {
  const g: SyncGroups = { create: [], update: [], delete: [], cleanup: [], adopt: [], baseline: [], deletedInSling: [], skip: [] };
  for (const a of actions) {
    if (a.kind === "skip" && a.skip_outcome === "skipped_missing") g.deletedInSling.push(a);
    else g[a.kind].push(a);
  }
  return g;
}

/** Actions that change Sling. */
export function slingChangeCount(p: SyncPreview): number {
  return p.actions.filter((a) => a.kind === "create" || a.kind === "update" || a.kind === "delete" || a.kind === "cleanup").length;
}

/** Whether confirming does anything at all: a Sling change, or bookkeeping
 *  (adopting another draft's identical shift, or recording a skip so e.g. a
 *  shift deleted in Sling is re-created next time). */
export function hasWork(p: SyncPreview): boolean {
  return p.actions.some((a) => a.kind !== "skip" || a.skip_outcome != null);
}

export function shiftLabel(v: ShiftView, fmt?: TimeFormat): string {
  return `${formatDayShort(v.date)} ${formatTime(v.start, fmt)} ${v.class_name} — ${v.teacher_name}`;
}

/** "Alex → Kay", "9:00 – 9:50 AM → 9:30 – 10:20 AM", "Classic → Define". */
export function updateLabel(a: SyncAction, fmt?: TimeFormat): string {
  const b = a.before;
  const n = a.after;
  if (!b || !n) return "";
  const parts: string[] = [];
  if (b.teacher_name !== n.teacher_name) parts.push(`${b.teacher_name} → ${n.teacher_name}`);
  if (b.date !== n.date || b.start !== n.start || b.end !== n.end)
    parts.push(`${formatTimeRange(b.start, b.end, fmt)} → ${formatTimeRange(n.start, n.end, fmt)}`);
  if (b.class_name !== n.class_name) parts.push(`${b.class_name} → ${n.class_name}`);
  return parts.join(", ");
}

export const SYNC_KIND_LABEL: Record<SyncActionKind, string> = {
  create: "Create",
  update: "Update",
  delete: "Remove",
  cleanup: "Remove (earlier draft)",
  adopt: "Keep (already in Sling)",
  baseline: "Now tracked (pushed before sync tracking)",
  skip: "Skipped",
};

/** Push button label: a first push creates; after that it syncs changes. */
export function pushLabel(s: Pick<ProposalSummary, "sling_shift_count">): string {
  return s.sling_shift_count > 0 ? "Sync to Sling" : "Push to Sling";
}

export const CONFLICT_KIND_LABEL: Record<DraftConflictKind, string> = {
  blocked: "Unavailable",
  leave: "On leave",
  teacher_inactive: "Deactivated",
  not_qualified: "Not qualified",
  over_cap: "Over weekly cap",
  unassigned: "Unassigned",
};
