import { describe, it, expect } from "vitest";
import { computeKpis } from "./kpis";
import type { ProposalShiftRow } from "../types";

let nextId = 1;
function shift(over: Partial<ProposalShiftRow>): ProposalShiftRow {
  return {
    id: nextId++,
    shift_date: "2026-08-03",
    start_time: "09:00",
    end_time: "10:00",
    class_name: "Classic",
    sling_position_id: 101,
    teacher_name: "Alex Braun",
    sling_user_id: 1,
    generation_reason: "test",
    flag: null,
    is_coteach: false,
    coteach_label: null,
    is_dropped: false,
    ...over,
  };
}

describe("computeKpis", () => {
  it("computes coverage as assigned share of non-dropped shifts", () => {
    const k = computeKpis([
      shift({}),
      shift({ sling_user_id: null, teacher_name: null }),
      shift({ sling_user_id: 2, teacher_name: "Kayla Moore" }),
      shift({ is_dropped: true, sling_user_id: null, teacher_name: null }),
    ]);
    expect(k.totalCount).toBe(3);
    expect(k.assignedCount).toBe(2);
    expect(k.coveragePct).toBe(67);
  });

  it("counts a co-teach row (no single sling_user_id) as assigned", () => {
    const k = computeKpis([
      shift({ sling_user_id: null, teacher_name: null, is_coteach: true, coteach_label: "Alex + Kayla" }),
    ]);
    expect(k.assignedCount).toBe(1);
    expect(k.coveragePct).toBe(100);
  });

  it("sums teacher hours from assigned, non-dropped shift durations", () => {
    const k = computeKpis([
      shift({ start_time: "09:00", end_time: "10:00" }), // 1h
      shift({ start_time: "17:30", end_time: "18:15" }), // 0.75h
      shift({ start_time: "05:45", end_time: "06:45", is_dropped: true }), // dropped — excluded
      shift({ start_time: "07:00", end_time: "08:00", sling_user_id: null, teacher_name: null }), // unassigned — excluded
    ]);
    expect(k.teacherHours).toBe(2); // 1.75 rounded
  });

  it("counts distinct assigned teachers", () => {
    const k = computeKpis([
      shift({ sling_user_id: 1 }),
      shift({ sling_user_id: 1 }),
      shift({ sling_user_id: 2, teacher_name: "Kayla Moore" }),
    ]);
    expect(k.teacherCount).toBe(2);
  });

  it("labels an even distribution Even", () => {
    const k = computeKpis([
      shift({ sling_user_id: 1 }),
      shift({ sling_user_id: 1 }),
      shift({ sling_user_id: 2 }),
      shift({ sling_user_id: 2 }),
      shift({ sling_user_id: 3 }),
      shift({ sling_user_id: 3 }),
    ]);
    expect(k.balance).toBe("Even");
  });

  it("labels a skewed distribution Uneven", () => {
    const k = computeKpis([
      ...Array.from({ length: 9 }, () => shift({ sling_user_id: 1 })),
      shift({ sling_user_id: 2 }),
    ]);
    expect(k.balance).toBe("Uneven");
  });

  it("returns em-dash balance and zero coverage with no shifts", () => {
    const k = computeKpis([]);
    expect(k.balance).toBe("—");
    expect(k.coveragePct).toBe(0);
    expect(k.teacherHours).toBe(0);
  });
});
