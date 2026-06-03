import { describe, it, expect } from "vitest";
import { computeIssues } from "./issues";
import type { ProposalShiftRow, Teacher } from "../types";

const teacher = (
  id: number,
  name: string,
  weekly_max: number,
): Teacher => ({
  sling_user_id: id,
  display_name: name,
  weekly_target: weekly_max,
  weekly_max,
  is_lead: false,
  ranking_weight: 1,
  variety_multiplier: 1,
  active: true,
  notes: null,
  locations: null,
});

const shift = (over: Partial<ProposalShiftRow>): ProposalShiftRow => ({
  id: 1,
  shift_date: "2026-06-01",
  start_time: "05:00",
  end_time: "06:00",
  class_name: "Classic",
  sling_position_id: 100,
  teacher_name: "Teacher A",
  sling_user_id: 1,
  generation_reason: "seed",
  flag: null,
  is_coteach: false,
  coteach_label: null,
  is_dropped: false,
  ...over,
});

describe("computeIssues", () => {
  it("flags unassigned slots", () => {
    const s = shift({ id: 1, teacher_name: null, sling_user_id: null });
    const w = computeIssues([s], [], new Set(), [], [], []);
    expect(w).toEqual([
      {
        kind: "unassigned",
        shift_id: 1,
        shift_date: "2026-06-01",
        message: expect.stringContaining("unassigned"),
      },
    ]);
  });

  it("does not flag dropped slots as unassigned", () => {
    const s = shift({ teacher_name: null, sling_user_id: null, is_dropped: true });
    expect(computeIssues([s], [], new Set(), [], [], [])).toEqual([]);
  });

  it("flags teachers over their weekly cap", () => {
    const t = teacher(1, "Teacher A", 5);
    const shifts = Array.from({ length: 6 }, (_, i) =>
      shift({ id: i + 1, shift_date: `2026-06-0${i + 1}` }),
    );
    const w = computeIssues(shifts, [t], new Set(), [], [], []);
    const cap = w.find((x) => x.kind === "over_cap");
    expect(cap).toBeDefined();
    expect(cap!.message).toContain("Teacher A");
    expect(cap!.message).toContain("6");
    expect(cap!.message).toContain("5");
  });

  it("excludes dropped shifts from cap counts", () => {
    const t = teacher(1, "Teacher A", 5);
    const shifts = Array.from({ length: 6 }, (_, i) =>
      shift({ id: i + 1, shift_date: `2026-06-0${i + 1}`, is_dropped: i === 0 }),
    );
    const overCap = computeIssues(shifts, [t], new Set(), [], [], []).filter((w) => w.kind === "over_cap");
    expect(overCap).toEqual([]);
  });

  it("flags qualification mismatches", () => {
    const t = teacher(1, "Teacher A", 5);
    const s = shift({ sling_user_id: 1, sling_position_id: 100 });
    // qualifiedPairs is empty -> Teacher A is not qualified for position 100
    const w = computeIssues([s], [t], new Set(), [], [], []);
    const q = w.find((x) => x.kind === "qualification");
    expect(q).toBeDefined();
    expect(q!.message).toContain("Teacher A");
  });

  it("does not flag qualification when the pair is in the set", () => {
    const t = teacher(1, "Teacher A", 5);
    const s = shift({ sling_user_id: 1, sling_position_id: 100 });
    const qualified = new Set(["1:100"]);
    expect(computeIssues([s], [t], qualified, [], [], [])).toEqual([]);
  });
});

describe("computeIssues — leave_conflict", () => {
  it("flags an overlapping leave block", () => {
    const t = teacher(1, "A", 5);
    const s = shift({ sling_user_id: 1, shift_date: "2026-06-01", start_time: "05:00", end_time: "06:00" });
    const blocks = [{ sling_user_id: 1, source: "leave",
      starts_at: "2026-06-01T04:00:00", ends_at: "2026-06-01T07:00:00" }];
    const out = computeIssues([s], [t], new Set(["1:100"]), blocks, [], []);
    expect(out.some((x) => x.kind === "leave_conflict")).toBe(true);
  });
});

describe("computeIssues — teacher_deactivated", () => {
  it("flags assignments of deactivated teachers", () => {
    const t = { ...teacher(1, "A", 5), active: false };
    const s = shift({ sling_user_id: 1 });
    const out = computeIssues([s], [t], new Set(["1:100"]), [], [], []);
    expect(out.some((x) => x.kind === "teacher_deactivated")).toBe(true);
  });
});

describe("computeIssues — external_shift", () => {
  it("flags a Sling shift not present in proposal", () => {
    const ext = [{ sling_shift_id: 99, shift_date: "2026-06-09", start_time: "05:45",
                   sling_user_id: 1001, sling_position_id: 29303965 }];
    const out = computeIssues([], [], new Set(), [], ext, []);
    expect(out.some((x) => x.kind === "external_shift")).toBe(true);
  });
  it("does not flag a Sling shift that matches a proposal slot", () => {
    const s = shift({ id: 1, shift_date: "2026-06-09", start_time: "05:45",
                      sling_user_id: 1001, sling_position_id: 29303965 });
    const ext = [{ sling_shift_id: 99, shift_date: "2026-06-09", start_time: "05:45",
                   sling_user_id: 1001, sling_position_id: 29303965 }];
    const out = computeIssues([s], [], new Set(), [], ext, []);
    expect(out.some((x) => x.kind === "external_shift")).toBe(false);
  });
  it("does not flag a Sling shift when proposal has any shift at the same date/time/position (even with different user)", () => {
    const s = shift({ id: 1, shift_date: "2026-06-09", start_time: "05:45",
                      sling_user_id: 1, sling_position_id: 29303965 });
    const ext = [{ sling_shift_id: 99, shift_date: "2026-06-09", start_time: "05:45",
                   sling_user_id: 1001, sling_position_id: 29303965 }];
    const out = computeIssues([s], [], new Set(), [], ext, []);
    expect(out.some((x) => x.kind === "external_shift")).toBe(false);
  });
});

describe("computeIssues — new_teacher", () => {
  it("emits a card per new user", () => {
    const out = computeIssues([], [], new Set(), [], [],
      [{ sling_user_id: 999, display_name: "Newbie" }]);
    expect(out.some((x) => x.kind === "new_teacher" && x.ref === 999)).toBe(true);
  });
});
