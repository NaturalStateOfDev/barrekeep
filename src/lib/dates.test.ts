import { describe, it, expect } from "vitest";
import {
  buildMonthGrid,
  isoWeekKey,
  initials,
  monthWindow,
  isReadOnlyMonth,
  formatTime,
  formatTimeShort,
  formatTimeRange,
  formatTimestamp,
  formatLocalDate,
  formatTimesInText,
  formatSlotLabel,
} from "./dates";

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
  it("converts instants (UTC Z or any offset) to studio wall-clock time", () => {
    // Backend form: ISO UTC with Z. Aug = CDT (-05:00), Nov 2 = CST (-06:00).
    expect(wallClock("2026-08-20T13:00:00Z")).toBe("2026-08-20T08:00:00");
    expect(wallClock("2026-11-02T11:00:00Z")).toBe("2026-11-02T05:00:00");
    // Across midnight UTC: still the previous studio day.
    expect(wallClock("2026-10-01T00:30:00Z")).toBe("2026-09-30T19:30:00");
    // Older DuckDB cast forms.
    expect(wallClock("2026-08-20 08:00:00-05")).toBe("2026-08-20T08:00:00");
    expect(wallClock("2026-08-20 13:00:00+00")).toBe("2026-08-20T08:00:00");
    expect(wallClock("2026-08-20 08:00:00.123-05:00")).toBe("2026-08-20T08:00:00");
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

describe("time display", () => {
  it("formatTime renders 12-hour by default", () => {
    expect(formatTime("00:00")).toBe("12:00 AM");
    expect(formatTime("12:00")).toBe("12:00 PM");
    expect(formatTime("09:05")).toBe("9:05 AM");
    expect(formatTime("23:59")).toBe("11:59 PM");
    expect(formatTime("13:30:00")).toBe("1:30 PM");
  });

  it("formatTime passes 24-hour through as HH:MM", () => {
    expect(formatTime("09:05", "24h")).toBe("09:05");
    expect(formatTime("23:59", "24h")).toBe("23:59");
    expect(formatTime("7:30", "24h")).toBe("07:30");
    expect(formatTime("13:30:00", "24h")).toBe("13:30");
  });

  it("returns unparseable input unchanged", () => {
    expect(formatTime("")).toBe("");
    expect(formatTime("soon")).toBe("soon");
    expect(formatTime("25:00")).toBe("25:00");
  });

  it("formatTimeShort is the compact form", () => {
    expect(formatTimeShort("05:45")).toBe("5:45a");
    expect(formatTimeShort("13:00")).toBe("1:00p");
    expect(formatTimeShort("00:10")).toBe("12:10a");
    expect(formatTimeShort("05:45", "24h")).toBe("05:45");
  });

  it("formatTimeRange collapses a shared AM/PM", () => {
    expect(formatTimeRange("09:45", "10:35")).toBe("9:45 – 10:35 AM");
    expect(formatTimeRange("11:30", "12:20")).toBe("11:30 AM – 12:20 PM");
    expect(formatTimeRange("17:30", "18:20")).toBe("5:30 – 6:20 PM");
    expect(formatTimeRange("09:45", "10:35", "12h", true)).toBe("9:45a–10:35a");
    expect(formatTimeRange("09:45", "10:35", "24h")).toBe("09:45–10:35");
  });

  it("formatTimestamp parses UTC 'Z' strings and shows them in the given zone", () => {
    const chi = "America/Chicago";
    expect(formatTimestamp("2026-07-03T14:20:44Z", "12h", chi)).toBe("2026-07-03 9:20 AM");
    expect(formatTimestamp("2026-07-03T14:20:44Z", "24h", chi)).toBe("2026-07-03 09:20:44");
    expect(formatTimestamp("2026-07-03T14:20:44Z", "24h", "UTC")).toBe("2026-07-03 14:20:44");
    // Evening Central = next day UTC; the local date is shown.
    expect(formatTimestamp("2026-10-01T00:30:00Z", "12h", chi)).toBe("2026-09-30 7:30 PM");
    // CST (-06:00) after fall-back.
    expect(formatTimestamp("2026-11-01T18:00:00Z", "12h", chi)).toBe("2026-11-01 12:00 PM");
    // Older forms still parse.
    expect(formatTimestamp("2026-07-03 14:20:44+00", "24h", chi)).toBe("2026-07-03 09:20:44");
    expect(formatTimestamp("2026-07-03 09:20:44.5-05", "24h", chi)).toBe("2026-07-03 09:20:44");
    expect(formatTimestamp("not a date")).toBe("not a date");
  });

  it("formatTimestamp treats offset-less strings as local wall time", () => {
    // Local in, local out, whatever the machine's zone.
    expect(formatTimestamp("2026-07-03T21:05:00", "12h")).toBe("2026-07-03 9:05 PM");
    expect(formatTimestamp("2026-07-03 21:05:00", "24h")).toBe("2026-07-03 21:05:00");
  });

  it("formatLocalDate gives the calendar date in the given zone", () => {
    expect(formatLocalDate("2026-10-01T00:30:00Z", "America/Chicago")).toBe("2026-09-30");
    expect(formatLocalDate("2026-10-01T00:30:00Z", "UTC")).toBe("2026-10-01");
  });

  it("formatTimesInText rewrites times inside app messages only in 12h", () => {
    const msg = "Alex is marked unavailable (05:00–06:00) — overlaps 05:45 Classic";
    expect(formatTimesInText(msg)).toBe(
      "Alex is marked unavailable (5:00 AM–6:00 AM) — overlaps 5:45 AM Classic",
    );
    expect(formatTimesInText(msg, "24h")).toBe(msg);
    expect(formatTimesInText("cap (3 / 2) at 2026-08-01")).toBe("cap (3 / 2) at 2026-08-01");
  });

  it("formatSlotLabel formats the time of a weekday slot", () => {
    expect(formatSlotLabel("Mon 17:30")).toBe("Mon 5:30 PM");
    expect(formatSlotLabel("Mon 17:30", "24h")).toBe("Mon 17:30");
    expect(formatSlotLabel("")).toBe("");
  });
});
