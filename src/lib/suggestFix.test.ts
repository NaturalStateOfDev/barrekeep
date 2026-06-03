import { describe, it, expect } from "vitest";
import { suggestSwap } from "./suggestFix";
import type { ProposalShiftRow, Teacher } from "../types";

const teacher = (id: number, name: string, max: number, weight = 1.0): Teacher => ({
  sling_user_id: id, display_name: name,
  weekly_target: max, weekly_max: max,
  is_lead: false, ranking_weight: weight, variety_multiplier: 1,
  active: true, notes: null, locations: null,
});

const shift = (over: Partial<ProposalShiftRow>): ProposalShiftRow => ({
  id: 1, shift_date: "2026-06-01", start_time: "05:00", end_time: "06:00",
  class_name: "Classic", sling_position_id: 100,
  teacher_name: null, sling_user_id: null,
  generation_reason: "seed", flag: null,
  is_coteach: false, coteach_label: null, is_dropped: false,
  ...over,
});

describe("suggestSwap", () => {
  it("returns null when no candidates are qualified", () => {
    const target = shift({});
    expect(suggestSwap(target, [], [teacher(1, "A", 5)], new Set(), [])).toBeNull();
  });

  it("picks the only qualified, active, under-cap teacher", () => {
    const target = shift({});
    const t = teacher(1, "A", 5);
    const result = suggestSwap(target, [], [t], new Set(["1:100"]), []);
    expect(result?.sling_user_id).toBe(1);
  });

  it("prefers higher ranking_weight", () => {
    const target = shift({});
    const a = teacher(1, "A", 5, 1.0);
    const b = teacher(2, "B", 5, 2.0);
    const quals = new Set(["1:100", "2:100"]);
    const result = suggestSwap(target, [], [a, b], quals, []);
    expect(result?.sling_user_id).toBe(2);
  });

  it("excludes teachers already at weekly_max", () => {
    const target = shift({ shift_date: "2026-06-08" });
    const a = teacher(1, "A", 1);
    const existingShifts = [
      shift({ id: 99, sling_user_id: 1, shift_date: "2026-06-09" }),
    ];
    const result = suggestSwap(target, existingShifts, [a], new Set(["1:100"]), []);
    expect(result).toBeNull();
  });

  it("excludes inactive teachers", () => {
    const target = shift({});
    const inactive = { ...teacher(1, "A", 5), active: false };
    expect(suggestSwap(target, [], [inactive], new Set(["1:100"]), [])).toBeNull();
  });

  it("excludes teachers with an overlapping availability block", () => {
    const target = shift({ shift_date: "2026-06-01", start_time: "05:00", end_time: "06:00" });
    const a = teacher(1, "A", 5);
    const blocks = [{
      sling_user_id: 1,
      source: "leave",
      starts_at: "2026-06-01T04:30:00-05:00",
      ends_at: "2026-06-01T07:30:00-05:00",
    }];
    const result = suggestSwap(target, [], [a], new Set(["1:100"]), blocks);
    expect(result).toBeNull();
  });
});
