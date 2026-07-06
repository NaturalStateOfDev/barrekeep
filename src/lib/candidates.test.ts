import { describe, it, expect } from "vitest";
import { candidatesFor } from "./candidates";
import type { ProposalShiftRow, Teacher, AvailabilityBlock } from "../types";

function teacher(over: Partial<Teacher>): Teacher {
  return {
    sling_user_id: 1,
    display_name: "Alex Braun",
    weekly_target: 4,
    weekly_max: 5,
    is_lead: false,
    ranking_weight: 1,
    variety_multiplier: 1,
    active: true,
    notes: null,
    locations: null,
    ...over,
  };
}

let nextId = 100;
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

const TEACHERS = [
  teacher({ sling_user_id: 1, display_name: "Alex Braun", ranking_weight: 2 }),
  teacher({ sling_user_id: 2, display_name: "Kayla Moore" }),
  teacher({ sling_user_id: 3, display_name: "Casey Diaz" }),
  teacher({ sling_user_id: 4, display_name: "Inactive Ida", active: false }),
];

// 1 and 2 qualified for position 101; 3 is not.
const PAIRS = new Set(["1:101", "2:101"]);

describe("candidatesFor", () => {
  it("excludes inactive teachers and lists the rest", () => {
    const target = shift({});
    const out = candidatesFor(target, [target], TEACHERS, PAIRS, []);
    expect(out.map((c) => c.teacher.sling_user_id)).not.toContain(4);
    expect(out).toHaveLength(3);
  });

  it("flags unqualified teachers with a note", () => {
    const target = shift({});
    const out = candidatesFor(target, [target], TEACHERS, PAIRS, []);
    const casey = out.find((c) => c.teacher.sling_user_id === 3)!;
    expect(casey.qualified).toBe(false);
    expect(casey.note).toBe("not qualified");
  });

  it("flags teachers on leave during the slot", () => {
    const target = shift({});
    const blocks: AvailabilityBlock[] = [
      { sling_user_id: 2, source: "leave", starts_at: "2026-08-03T08:00:00", ends_at: "2026-08-03T12:00:00" },
    ];
    const out = candidatesFor(target, [target], TEACHERS, PAIRS, blocks);
    const kayla = out.find((c) => c.teacher.sling_user_id === 2)!;
    expect(kayla.note).toBe("on leave");
  });

  it("detects same-day leave in the backend's TIMESTAMPTZ cast format", () => {
    // The Rust command returns 'YYYY-MM-DD HH:MM:SS±TZ' (space + offset).
    const target = shift({});
    const blocks: AvailabilityBlock[] = [
      { sling_user_id: 2, source: "leave", starts_at: "2026-08-03 08:00:00-05", ends_at: "2026-08-03 12:00:00-05" },
    ];
    const out = candidatesFor(target, [target], TEACHERS, PAIRS, blocks);
    const kayla = out.find((c) => c.teacher.sling_user_id === 2)!;
    expect(kayla.note).toBe("on leave");
  });

  it("flags teachers whose week is at cap", () => {
    const target = shift({ sling_user_id: null, teacher_name: null });
    // Kayla already teaches 5 classes (her weekly_max) that week.
    const week = ["2026-08-03", "2026-08-04", "2026-08-05", "2026-08-06", "2026-08-07"].map((d) =>
      shift({ shift_date: d, sling_user_id: 2, teacher_name: "Kayla Moore" }),
    );
    const out = candidatesFor(target, [target, ...week], TEACHERS, PAIRS, []);
    const kayla = out.find((c) => c.teacher.sling_user_id === 2)!;
    expect(kayla.note).toBe("at weekly cap");
  });

  it("does not count the target slot against the current teacher's cap note", () => {
    // Alex teaches exactly weekly_max classes including this one; reassigning
    // to Alex changes nothing, so no cap note.
    const target = shift({ sling_user_id: 1 });
    const others = ["2026-08-04", "2026-08-05", "2026-08-06", "2026-08-07"].map((d) =>
      shift({ shift_date: d, sling_user_id: 1 }),
    );
    const alexMax5 = [teacher({ sling_user_id: 1, display_name: "Alex Braun", weekly_max: 5 })];
    const out = candidatesFor(target, [target, ...others], alexMax5, PAIRS, []);
    const alex = out.find((c) => c.teacher.sling_user_id === 1)!;
    expect(alex.current).toBe(true);
    expect(alex.note).toBeNull();
  });

  it("sorts clean qualified candidates first, unqualified last", () => {
    const target = shift({ sling_user_id: null, teacher_name: null });
    const blocks: AvailabilityBlock[] = [
      { sling_user_id: 2, source: "leave", starts_at: "2026-08-03T08:00:00", ends_at: "2026-08-03T12:00:00" },
    ];
    const out = candidatesFor(target, [target], TEACHERS, PAIRS, blocks);
    expect(out[0].teacher.sling_user_id).toBe(1); // qualified, no note
    expect(out[out.length - 1].qualified).toBe(false); // unqualified last
  });
});
