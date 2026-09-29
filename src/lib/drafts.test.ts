import { describe, it, expect } from "vitest";
import {
  consistencyStats,
  defaultComparePair,
  draftsForMonth,
  estimateCostLabel,
  filterDiff,
  MAX_DRAFT_NAME,
  pushDraftFor,
  representativeDraft,
  whatIfName,
} from "./drafts";
import type { DraftSlotDiff, ProposalShiftRow, ProposalSummary } from "../types";

function draft(over: Partial<ProposalSummary>): ProposalSummary {
  return {
    id: 1,
    target_month: "2026-08",
    algorithm_version: "v9",
    generated_at: "2026-07-01 10:00:00",
    is_current: false,
    shift_count: 0,
    dropped_count: 0,
    edit_count: 0,
    name: "Draft 1",
    archived: false,
    parent_proposal_id: null,
    created_from: "generate",
    is_push_candidate: false,
    pushed: false,
    ...over,
  };
}

// id DESC, as list_proposals returns them.
const LIST = [
  draft({ id: 9, name: "What-if", archived: true }),
  draft({ id: 8, name: "Consistent days" }),
  draft({ id: 7, name: "Draft 1", is_push_candidate: true }),
  draft({ id: 5, target_month: "2026-07", name: "Draft 1", is_push_candidate: true }),
];

describe("draft selection", () => {
  it("hides archived drafts unless asked, or unless it is the one being viewed", () => {
    expect(draftsForMonth(LIST, "2026-08", false).map((d) => d.id)).toEqual([8, 7]);
    expect(draftsForMonth(LIST, "2026-08", true).map((d) => d.id)).toEqual([9, 8, 7]);
    expect(draftsForMonth(LIST, "2026-08", false, 9).map((d) => d.id)).toEqual([9, 8, 7]);
  });

  it("finds the push draft and prefers it as the month's representative", () => {
    expect(pushDraftFor(LIST, "2026-08")?.id).toBe(7);
    expect(representativeDraft(LIST, "2026-08")?.id).toBe(7);
    const noPush = LIST.map((d) => ({ ...d, is_push_candidate: false }));
    expect(representativeDraft(noPush, "2026-08")?.id).toBe(8); // newest non-archived
    expect(representativeDraft(LIST, "2026-10")).toBeUndefined();
  });

  it("compares the push draft against the viewed draft by default", () => {
    const aug = draftsForMonth(LIST, "2026-08", false);
    expect(defaultComparePair(aug, 8)).toEqual([7, 8]);
    expect(defaultComparePair(aug, 7)).toEqual([7, 8]);
    expect(defaultComparePair([aug[0]], 8)).toBeNull();
  });
});

describe("whatIfName", () => {
  it("suffixes and stays within the name limit", () => {
    expect(whatIfName("Draft 2")).toBe("Draft 2 · what-if");
    const long = whatIfName("x".repeat(80));
    expect(long.length).toBeLessThanOrEqual(MAX_DRAFT_NAME);
    expect(long.endsWith("· what-if")).toBe(true);
  });
});

describe("filterDiff", () => {
  const rows: DraftSlotDiff[] = [
    { date: "2026-08-04", weekday: "Tue", start: "08:45", class_a: "Classic", class_b: "Classic", teacher_a: "Jane Roe", teacher_b: "Kay M", kind: "teacher" },
    { date: "2026-08-05", weekday: "Wed", start: "17:30", class_a: "Define", class_b: "Classic", teacher_a: "Kay M", teacher_b: "Kay M", kind: "format" },
    { date: "2026-08-08", weekday: "Sat", start: "10:00", class_a: null, class_b: "Focus", teacher_a: null, teacher_b: "Jane Roe + Alex T", kind: "only_b" },
  ];
  it("filters by weekday and by teacher on either side, incl. co-teach labels", () => {
    expect(filterDiff(rows, { teacher: "", weekday: "" })).toHaveLength(3);
    expect(filterDiff(rows, { teacher: "", weekday: "Wed" }).map((r) => r.start)).toEqual(["17:30"]);
    expect(filterDiff(rows, { teacher: "Jane Roe", weekday: "" }).map((r) => r.weekday)).toEqual(["Tue", "Sat"]);
    expect(filterDiff(rows, { teacher: "Jane Roe", weekday: "Sat" })).toHaveLength(1);
  });
});

describe("estimateCostLabel", () => {
  it("scales with the number of drafts", () => {
    expect(estimateCostLabel(null, 1)).toBe("1 Claude call (one per draft)");
    expect(estimateCostLabel(null, 3)).toContain("about 3×");
    expect(estimateCostLabel(0.12, 3)).toContain("$0.36");
    expect(estimateCostLabel(0.12, 0)).toBe("Pick at least one draft.");
  });
});

describe("consistencyStats", () => {
  let id = 1;
  const s = (date: string, start: string, uid: number | null, dropped = false): ProposalShiftRow => ({
    id: id++,
    shift_date: date,
    start_time: start,
    end_time: "23:00",
    class_name: "Classic",
    sling_position_id: 101,
    teacher_name: null,
    sling_user_id: uid,
    generation_reason: "t",
    flag: null,
    is_coteach: false,
    coteach_label: null,
    is_dropped: dropped,
  });

  it("counts distinct weekday+time slots and the top slot share", () => {
    // 2026-08-03/10/17 are Mondays, 2026-08-04 a Tuesday.
    const stats = consistencyStats([
      s("2026-08-03", "09:00", 1),
      s("2026-08-10", "09:00", 1),
      s("2026-08-04", "17:30", 1),
      s("2026-08-17", "09:00", 2),
      s("2026-08-04", "08:45", 2),
      s("2026-08-17", "10:00", null, true),
    ]);
    expect(stats.get(1)).toEqual({ classes: 3, distinct_slots: 2, top_slot: "Mon 09:00", top_slot_count: 2, top_slot_share: 2 / 3 });
    // Tie between Mon 09:00 and Tue 08:45 -> earliest weekday wins (as in Rust).
    expect(stats.get(2)?.top_slot).toBe("Mon 09:00");
    expect(stats.size).toBe(2);
  });
});
