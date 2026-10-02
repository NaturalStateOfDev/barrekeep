import { describe, it, expect } from "vitest";
import {
  availabilitySummary,
  blockKind,
  blockLabel,
  blocksOnDate,
  blockTag,
  buildAvailabilityGrid,
  cellStatus,
  describeCell,
  describeDayBlock,
  isRecurring,
  monthDates,
  slotAvailability,
  slotInWindows,
  strongestKind,
} from "./availability";
import { emptyWeek, hoursError, normalizeWeek } from "./studioHours";
import type { AvailabilityBlock, AvailabilityWindow, DayRange, Teacher } from "../types";

const teacher = (id: number, name: string, active = true): Teacher => ({
  sling_user_id: id,
  display_name: name,
  weekly_target: 4,
  weekly_max: 5,
  is_lead: false,
  ranking_weight: 1,
  variety_multiplier: 1,
  active,
  notes: null,
  locations: null,
});

const block = (uid: number, source: string, starts_at: string, ends_at: string): AvailabilityBlock => ({
  sling_user_id: uid,
  source,
  starts_at,
  ends_at,
});

const win = (uid: number, date: string, start: string, end: string): AvailabilityWindow => ({
  sling_user_id: uid,
  date,
  start,
  end,
});

const range = (date: string, open = "05:30", close = "19:30", widened = false): DayRange => ({
  date,
  open,
  close,
  widened,
});

describe("block labels", () => {
  it("calls Sling availability entries Unavailable, never leave", () => {
    expect(blockKind("availability")).toBe("unavailable");
    expect(blockKind("availability_set")).toBe("unavailable");
    expect(blockKind("availability_set_pending")).toBe("pending");
    expect(blockKind("leave")).toBe("leave");
    expect(blockLabel("availability")).toBe("Unavailable");
    expect(blockLabel("availability_set")).toBe("Unavailable");
    expect(blockLabel("availability_set_pending")).toBe("Unavailable (pending approval)");
    expect(blockLabel("leave")).toBe("On leave");
  });

  it("treats an unknown source as unavailable rather than ignoring it", () => {
    expect(blockKind("something_new")).toBe("unavailable");
    expect(blockLabel("something_new")).toBe("Unavailable");
  });

  it("has short tags for the day editor", () => {
    expect(blockTag("leave")).toBe("on leave");
    expect(blockTag("unavailable")).toBe("unavailable");
    expect(blockTag("pending")).toBe("unavailable (pending)");
  });

  it("knows which sources are recurring sets", () => {
    expect(isRecurring("availability_set")).toBe(true);
    expect(isRecurring("availability_set_pending")).toBe(true);
    expect(isRecurring("availability")).toBe(false);
    expect(isRecurring("leave")).toBe(false);
  });

  it("picks the strongest reason among overlapping blocks", () => {
    const b = (s: string) => block(1, s, "2026-11-03T09:00:00", "2026-11-03T10:00:00");
    expect(strongestKind([])).toBeNull();
    expect(strongestKind([b("availability_set_pending")])).toBe("pending");
    expect(strongestKind([b("availability_set_pending"), b("availability_set")])).toBe("unavailable");
    expect(strongestKind([b("availability"), b("leave")])).toBe("leave");
  });

  it("words the pull summary as unavailability blocks and leave days", () => {
    expect(availabilitySummary(12, 3)).toBe("12 unavailability blocks, 3 leave days");
    expect(availabilitySummary(1, 1)).toBe("1 unavailability block, 1 leave day");
    expect(availabilitySummary(0, 0)).toBe("0 unavailability blocks, 0 leave days");
  });
});

describe("slotAvailability", () => {
  const windows = [win(1, "2026-11-03", "05:30", "09:45"), win(1, "2026-11-03", "10:45", "19:30")];
  const avail = { windows, day_ranges: [range("2026-11-03")] };

  it("checks a slot against the teacher's windows", () => {
    expect(slotInWindows(windows, 1, "2026-11-03", "05:45", "06:45")).toBe(true);
    expect(slotInWindows(windows, 1, "2026-11-03", "10:45", "11:45")).toBe(true);
    expect(slotInWindows(windows, 1, "2026-11-03", "09:45", "10:45")).toBe(false);
    expect(slotInWindows(windows, 1, "2026-11-03", "09:00", "10:00")).toBe(false);
    expect(slotInWindows(windows, 2, "2026-11-03", "05:45", "06:45")).toBe(false);
    expect(slotInWindows(windows, 1, "2026-11-04", "05:45", "06:45")).toBe(false);
  });

  it("names the block that makes a teacher unavailable", () => {
    // Blocks arrive as UTC instants: 15:45Z = 9:45 AM Central (CST).
    const blocks = [block(1, "availability_set", "2026-11-03T15:45:00Z", "2026-11-03T16:45:00Z")];
    expect(slotAvailability(blocks, avail, 1, "2026-11-03", "09:45", "10:45")).toEqual({
      free: false,
      reason: "unavailable",
    });
    expect(slotAvailability(blocks, avail, 1, "2026-11-03", "10:45", "11:45")).toEqual({ free: true, reason: null });
    const leave = [block(1, "leave", "2026-11-03T06:00:00Z", "2026-11-04T06:00:00Z")];
    expect(slotAvailability(leave, null, 1, "2026-11-03", "09:45", "10:45").reason).toBe("leave");
    const pending = [block(1, "availability_set_pending", "2026-11-03T15:45:00Z", "2026-11-03T16:45:00Z")];
    expect(slotAvailability(pending, null, 1, "2026-11-03", "09:45", "10:45").reason).toBe("pending");
  });

  it("requires a window when availability is loaded", () => {
    // No block at all, but the teacher has no window covering the slot.
    expect(slotAvailability([], avail, 1, "2026-11-03", "09:45", "10:45")).toEqual({
      free: false,
      reason: "unavailable",
    });
    // Teacher 2 has no windows that day at all.
    expect(slotAvailability([], avail, 2, "2026-11-03", "05:45", "06:45").free).toBe(false);
  });

  it("falls back to blocks when availability is missing or doesn't cover the slot", () => {
    expect(slotAvailability([], null, 1, "2026-11-03", "09:45", "10:45").free).toBe(true);
    expect(slotAvailability([], undefined, 1, "2026-11-03", "09:45", "10:45").free).toBe(true);
    // A class just moved outside the computed span: the span is stale.
    expect(slotAvailability([], avail, 1, "2026-11-03", "19:00", "20:00").free).toBe(true);
    // A date with no span (closed, or not computed).
    expect(slotAvailability([], avail, 1, "2026-11-08", "09:00", "10:00").free).toBe(true);
  });
});

describe("availability grid", () => {
  it("lists every date of the month", () => {
    expect(monthDates("2026-11")).toHaveLength(30);
    expect(monthDates("2028-02")).toHaveLength(29);
    expect(monthDates("2026-11")[0]).toBe("2026-11-01");
    expect(monthDates("2026-12")[30]).toBe("2026-12-31");
  });

  it("clips a teacher's blocks to one date", () => {
    const blocks = [
      block(1, "leave", "2026-11-02T06:00:00Z", "2026-11-05T05:59:59Z"), // Nov 2 00:00 → Nov 4 23:59:59 CST
      block(1, "availability_set", "2026-11-03T15:45:00Z", "2026-11-03T16:45:00Z"),
      block(2, "availability", "2026-11-03T15:45:00Z", "2026-11-03T16:45:00Z"),
    ];
    const day = blocksOnDate(blocks, 1, "2026-11-03");
    expect(day).toEqual([
      { kind: "leave", recurring: false, start: "00:00", end: "24:00", allDay: true },
      { kind: "unavailable", recurring: true, start: "09:45", end: "10:45", allDay: false },
    ]);
    expect(blocksOnDate(blocks, 1, "2026-11-04")[0].allDay).toBe(true);
    expect(blocksOnDate(blocks, 1, "2026-11-05")).toEqual([]);
    expect(blocksOnDate(blocks, 1, "2026-11-01")).toEqual([]);
  });

  it("derives a cell's status", () => {
    const r = range("2026-11-03");
    const full = [win(1, "2026-11-03", "05:30", "19:30")];
    const part = [win(1, "2026-11-03", "05:30", "09:45"), win(1, "2026-11-03", "10:45", "19:30")];
    const b = (kind: "leave" | "unavailable" | "pending", start = "09:45", end = "10:45") => ({
      kind,
      recurring: false,
      start,
      end,
      allDay: false,
    });
    expect(cellStatus(null, [], [])).toBe("closed");
    expect(cellStatus(r, full, [])).toBe("available");
    // A block entirely outside the studio day changes nothing.
    expect(cellStatus(r, full, [b("unavailable", "21:00", "22:00")])).toBe("available");
    expect(cellStatus(r, part, [b("unavailable")])).toBe("partial");
    expect(cellStatus(r, part, [b("pending")])).toBe("pending");
    expect(cellStatus(r, part, [b("pending"), b("unavailable", "12:00", "13:00")])).toBe("partial");
    expect(cellStatus(r, [], [b("unavailable", "00:00", "24:00")])).toBe("unavailable");
    expect(cellStatus(r, [], [b("pending", "00:00", "24:00")])).toBe("pending");
    expect(cellStatus(r, [], [b("leave", "00:00", "24:00"), b("unavailable")])).toBe("leave");
  });

  it("builds a teacher × day grid", () => {
    const teachers = [teacher(2, "Zoe"), teacher(1, "Amy"), teacher(3, "Gone", false)];
    const availability = {
      day_ranges: [range("2026-11-02"), range("2026-11-03")],
      windows: [
        win(1, "2026-11-02", "05:30", "19:30"),
        win(1, "2026-11-03", "10:45", "19:30"),
        win(1, "2026-11-03", "05:30", "09:45"),
        win(2, "2026-11-02", "05:30", "19:30"),
      ],
    };
    const blocks = [
      block(1, "availability_set", "2026-11-03T15:45:00Z", "2026-11-03T16:45:00Z"),
      block(2, "leave", "2026-11-03T06:00:00Z", "2026-11-04T05:59:59Z"),
    ];
    const { dates, rows } = buildAvailabilityGrid("2026-11", teachers, availability, blocks);
    expect(dates).toHaveLength(30);
    expect(rows.map((r) => r.teacher.display_name)).toEqual(["Amy", "Zoe"]);
    const amy = rows[0];
    expect(amy.cells[0].status).toBe("closed"); // Nov 1: no span
    expect(amy.cells[1].status).toBe("available");
    expect(amy.cells[2].status).toBe("partial");
    expect(amy.cells[2].windows.map((w) => w.start)).toEqual(["05:30", "10:45"]);
    expect(amy.limitedDays).toBe(1);
    const zoe = rows[1];
    expect(zoe.cells[1].status).toBe("available");
    expect(zoe.cells[2].status).toBe("leave");
    expect(zoe.limitedDays).toBe(1);
  });

  it("describes a cell in the user's time format", () => {
    const cell = {
      date: "2026-11-03",
      status: "partial" as const,
      range: range("2026-11-03"),
      windows: [win(1, "2026-11-03", "05:30", "09:45"), win(1, "2026-11-03", "10:45", "19:30")],
      blocks: [
        { kind: "unavailable" as const, recurring: true, start: "09:45", end: "10:45", allDay: false },
        { kind: "pending" as const, recurring: true, start: "00:00", end: "24:00", allDay: true },
        { kind: "leave" as const, recurring: false, start: "14:00", end: "24:00", allDay: false },
      ],
    };
    expect(describeCell(cell, "12h")).toEqual([
      "Available 5:30 – 9:45 AM",
      "Available 10:45 AM – 7:30 PM",
      "Unavailable 9:45 – 10:45 AM · recurring",
      "Unavailable (pending approval) all day · recurring",
      "On leave from 2:00 PM",
    ]);
    expect(describeCell(cell, "24h")[0]).toBe("Available 05:30–09:45");
    expect(describeDayBlock({ kind: "leave", recurring: false, start: "00:00", end: "12:00", allDay: false }, "24h")).toBe(
      "until 12:00",
    );
    expect(describeCell({ ...cell, range: null, windows: [], blocks: [] })).toEqual([
      "Studio closed — no classes this day",
    ]);
    expect(describeCell({ ...cell, windows: [], blocks: [] })[0]).toBe("Not available this day");
  });
});

describe("studio hours form", () => {
  it("normalizes to seven days, Monday first", () => {
    const week = normalizeWeek([
      { weekday: 2, closed: false, open: "06:00", close: "19:00" },
      { weekday: 0, closed: true, open: "06:00", close: "19:00" },
    ]);
    expect(week).toHaveLength(7);
    expect(week[2]).toEqual({ weekday: 2, closed: false, open: "06:00", close: "19:00" });
    expect(week[0]).toEqual({ weekday: 0, closed: true, open: null, close: null });
    expect(emptyWeek().every((d) => d.closed)).toBe(true);
  });

  it("validates before saving", () => {
    const week = emptyWeek();
    expect(hoursError(week)).toBeNull();
    week[1] = { weekday: 1, closed: false, open: "06:00", close: "19:00" };
    expect(hoursError(week)).toBeNull();
    week[1] = { weekday: 1, closed: false, open: "19:00", close: "06:00" };
    expect(hoursError(week)).toMatch(/^Tuesday: the opening time must be before/);
    week[1] = { weekday: 1, closed: false, open: "06:00", close: null };
    expect(hoursError(week)).toMatch(/^Tuesday: set both/);
  });
});
