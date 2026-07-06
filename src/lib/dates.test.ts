import { describe, it, expect } from "vitest";
import { buildMonthGrid, isoWeekKey, initials, monthWindow, isReadOnlyMonth } from "./dates";

describe("buildMonthGrid", () => {
  it("produces 6 weeks of 7 days for June 2026", () => {
    const grid = buildMonthGrid("2026-06");
    expect(grid).toHaveLength(6);
    expect(grid[0]).toHaveLength(7);
  });

  it("starts the grid on a Sunday and includes leading days from May", () => {
    const grid = buildMonthGrid("2026-06");
    // June 1 2026 is a Monday; the grid's first row starts Sun May 31.
    expect(grid[0][0]).toEqual({ iso: "2026-05-31", inMonth: false });
    expect(grid[0][1]).toEqual({ iso: "2026-06-01", inMonth: true });
  });

  it("flags out-of-month days correctly", () => {
    const grid = buildMonthGrid("2026-06");
    const allInMonth = grid.flat().filter((d) => d.inMonth);
    expect(allInMonth).toHaveLength(30); // June has 30 days
  });
});

describe("isoWeekKey", () => {
  it("returns 2026-W23 for Mon Jun 1 2026", () => {
    expect(isoWeekKey("2026-06-01")).toBe("2026-W23");
  });

  it("groups Sun-Sat the same way as Mon-Sun (ISO weeks are Mon-Sun)", () => {
    // Sun Jun 7 is the last day of ISO week 23.
    expect(isoWeekKey("2026-06-07")).toBe("2026-W23");
    // Mon Jun 8 starts ISO week 24.
    expect(isoWeekKey("2026-06-08")).toBe("2026-W24");
  });
});

describe("initials", () => {
  it("returns first + last initials", () => {
    expect(initials("Teacher A")).toBe("TA");
  });

  it("uppercases", () => {
    expect(initials("teacher x")).toBe("TX");
  });

  it("handles single-word names", () => {
    expect(initials("Solo")).toBe("S");
  });

  it("returns ?? for null", () => {
    expect(initials(null)).toBe("??");
  });
});

describe("monthWindow", () => {
  it("returns prev + current + next 2 months", () => {
    expect(monthWindow("2026-05-19")).toEqual([
      "2026-04", "2026-05", "2026-06", "2026-07",
    ]);
  });
  it("rolls over across year boundary", () => {
    expect(monthWindow("2026-12-15")).toEqual([
      "2026-11", "2026-12", "2027-01", "2027-02",
    ]);
  });
  it("rolls back across year boundary", () => {
    expect(monthWindow("2026-01-05")).toEqual([
      "2025-12", "2026-01", "2026-02", "2026-03",
    ]);
  });
});

describe("isReadOnlyMonth", () => {
  it("flags past months as read-only", () => {
    expect(isReadOnlyMonth("2026-04", "2026-05-19")).toBe(true);
  });
  it("does not flag current month", () => {
    expect(isReadOnlyMonth("2026-05", "2026-05-19")).toBe(false);
  });
  it("does not flag future months", () => {
    expect(isReadOnlyMonth("2026-06", "2026-05-19")).toBe(false);
  });
});

import { wallClock } from "./dates";

describe("wallClock", () => {
  it("normalizes DuckDB TIMESTAMPTZ casts (space + offset)", () => {
    expect(wallClock("2026-08-20 08:00:00-05")).toBe("2026-08-20T08:00:00");
    expect(wallClock("2026-08-20 08:00:00+00")).toBe("2026-08-20T08:00:00");
    expect(wallClock("2026-08-20 08:00:00.123-05:30")).toBe("2026-08-20T08:00:00");
  });

  it("passes through shift-local ISO strings unchanged", () => {
    expect(wallClock("2026-08-20T05:45:00")).toBe("2026-08-20T05:45:00");
  });

  it("makes cross-format comparisons consistent", () => {
    // Same-day leave block vs shift: the raw strings compare wrongly
    // (' ' < 'T'), normalized they compare correctly.
    const blockEnd = wallClock("2026-08-20 12:00:00-05");
    const shiftStart = wallClock("2026-08-20T05:45:00");
    expect(blockEnd > shiftStart).toBe(true);
  });
});
