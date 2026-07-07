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
  teacher({ sling_user_id: 1, display_name: "Alex Braun" }),
  teacher({ sling_user_id: 2, display_name: "Kayla Moore" }),
  teacher({ sling_user_id: 3, display_name: "Casey Diaz" }),
  teacher({ sling_user_id: 4, display_name: "Inactive Ida", active: false }),
];

// 1 and 2 trained for position 101; 3 is not.
const PAIRS = new Set(["1:101", "2:101"]);

describe("candidatesFor", () => {
  it("lists the whole active roster and excludes deactivated teachers", () => {
    const target = shift({});
    const out = candidatesFor(target, [target], TEACHERS, PAIRS, []);
    expect(out.map((c) => c.teacher.sling_user_id)).not.toContain(4);
    expect(out).toHaveLength(3);
  });

  it("marks trained and available independently", () => {
    const target = shift({});
    const blocks: AvailabilityBlock[] = [
      { sling_user_id: 2, source: "leave", starts_at: "2026-08-03T08:00:00", ends_at: "2026-08-03T12:00:00" },
    ];
    const out = candidatesFor(target, [target], TEACHERS, PAIRS, blocks);
    const alex = out.find((c) => c.teacher.sling_user_id === 1)!;
    const kayla = out.find((c) => c.teacher.sling_user_id === 2)!;
    const casey = out.find((c) => c.teacher.sling_user_id === 3)!;
    expect(alex).toMatchObject({ qualified: true, on_leave: false, at_cap: false, available: true });
    expect(kayla).toMatchObject({ qualified: true, on_leave: true, available: false });
    expect(casey).toMatchObject({ qualified: false, available: true });
  });

  it("detects same-day leave in the backend's TIMESTAMPTZ cast format", () => {
    // The Rust command returns 'YYYY-MM-DD HH:MM:SS±TZ' (space + offset).
    const target = shift({});
    const blocks: AvailabilityBlock[] = [
      { sling_user_id: 2, source: "leave", starts_at: "2026-08-03 08:00:00-05", ends_at: "2026-08-03 12:00:00-05" },
    ];
    const out = candidatesFor(target, [target], TEACHERS, PAIRS, blocks);
    const kayla = out.find((c) => c.teacher.sling_user_id === 2)!;
    expect(kayla.on_leave).toBe(true);
  });

  it("marks teachers whose week is at cap as unavailable", () => {
    const target = shift({ sling_user_id: null, teacher_name: null });
    // Kayla already teaches 5 classes (her weekly_max) that week.
    const week = ["2026-08-03", "2026-08-04", "2026-08-05", "2026-08-06", "2026-08-07"].map((d) =>
      shift({ shift_date: d, sling_user_id: 2, teacher_name: "Kayla Moore" }),
    );
    const out = candidatesFor(target, [target, ...week], TEACHERS, PAIRS, []);
    const kayla = out.find((c) => c.teacher.sling_user_id === 2)!;
    expect(kayla.at_cap).toBe(true);
    expect(kayla.available).toBe(false);
  });

  it("does not count the target slot against the current teacher's cap", () => {
    // Alex teaches exactly weekly_max classes including this one; reassigning
    // to Alex changes nothing, so no cap mark.
    const target = shift({ sling_user_id: 1 });
    const others = ["2026-08-04", "2026-08-05", "2026-08-06", "2026-08-07"].map((d) =>
      shift({ shift_date: d, sling_user_id: 1 }),
    );
    const alexMax5 = [teacher({ sling_user_id: 1, display_name: "Alex Braun", weekly_max: 5 })];
    const out = candidatesFor(target, [target, ...others], alexMax5, PAIRS, []);
    const alex = out.find((c) => c.teacher.sling_user_id === 1)!;
    expect(alex.current).toBe(true);
    expect(alex.at_cap).toBe(false);
  });

  it("sorts trained first, then available, then alphabetically", () => {
    const roster = [
      teacher({ sling_user_id: 1, display_name: "Zoe Trained-Free" }),
      teacher({ sling_user_id: 2, display_name: "Amy Trained-Leave" }),
      teacher({ sling_user_id: 3, display_name: "Bea Trained-Free" }),
      teacher({ sling_user_id: 5, display_name: "Ann Untrained" }),
    ];
    const pairs = new Set(["1:101", "2:101", "3:101"]);
    const target = shift({ sling_user_id: null, teacher_name: null });
    const blocks: AvailabilityBlock[] = [
      { sling_user_id: 2, source: "leave", starts_at: "2026-08-03T08:00:00", ends_at: "2026-08-03T12:00:00" },
    ];
    const out = candidatesFor(target, [target], roster, pairs, blocks);
    expect(out.map((c) => c.teacher.display_name)).toEqual([
      "Bea Trained-Free",   // trained + available, alphabetical…
      "Zoe Trained-Free",
      "Amy Trained-Leave",  // trained but unavailable
      "Ann Untrained",      // untrained last
    ]);
  });
});
