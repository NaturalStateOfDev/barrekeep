// Availability helpers shared by the issue queue, the day editor, the fix
// suggester and the Availability view.
//
// Two views of the same facts:
//   - BLOCKS (`availability_blocks`): time a teacher is NOT available. Sling's
//     naming is backward — its "availability" entries are unavailability.
//     Sources: 'availability' (one-off, calendar feed), 'availability_set'
//     (an occurrence of a recurring set), 'availability_set_pending' (a
//     recurring set not yet approved in Sling — still blocked) and 'leave'.
//   - WINDOWS (`teacher_availability_windows`): when a teacher IS available —
//     the date's studio hours minus every block, computed by the backend.

import type {
  AvailabilityBlock,
  AvailabilityWindow,
  DayRange,
  MonthAvailability,
  Teacher,
} from "../types";
import { formatTime, formatTimeRange, wallClock, type TimeFormat } from "./dates";

export type BlockKind = "leave" | "unavailable" | "pending";

/** What a block source means to the scheduler. Unknown sources are treated
 *  as plain unavailability — a block is never ignored. */
export function blockKind(source: string): BlockKind {
  if (source === "leave") return "leave";
  if (source === "availability_set_pending") return "pending";
  return "unavailable";
}

/** True for blocks that come from a recurring Sling availability set. */
export function isRecurring(source: string): boolean {
  return source === "availability_set" || source === "availability_set_pending";
}

/** Title-case label: "On leave" / "Unavailable" / "Unavailable (pending approval)". */
export function blockLabel(source: string): string {
  switch (blockKind(source)) {
    case "leave":
      return "On leave";
    case "pending":
      return "Unavailable (pending approval)";
    default:
      return "Unavailable";
  }
}

/** Short lower-case tag for chips: "on leave" / "unavailable" / "unavailable (pending)". */
export function blockTag(kind: BlockKind): string {
  return kind === "leave" ? "on leave" : kind === "pending" ? "unavailable (pending)" : "unavailable";
}

/** A teacher's blocks overlapping a studio-local span
 *  ('YYYY-MM-DDTHH:MM:SS', exclusive ends). */
export function overlappingBlocks(
  blocks: AvailabilityBlock[],
  userId: number,
  startIso: string,
  endIso: string,
): AvailabilityBlock[] {
  return blocks.filter(
    (b) =>
      b.sling_user_id === userId &&
      wallClock(b.starts_at) < endIso &&
      wallClock(b.ends_at) > startIso,
  );
}

/** The kind that best explains a set of overlapping blocks: leave wins,
 *  then approved unavailability, then pending. null = no blocks. */
export function strongestKind(blocks: AvailabilityBlock[]): BlockKind | null {
  if (blocks.length === 0) return null;
  const kinds = new Set(blocks.map((b) => blockKind(b.source)));
  return kinds.has("leave") ? "leave" : kinds.has("unavailable") ? "unavailable" : "pending";
}

/** Is the slot inside one of the teacher's available windows that date? */
export function slotInWindows(
  windows: AvailabilityWindow[],
  userId: number,
  date: string,
  start: string,
  end: string,
): boolean {
  return windows.some(
    (w) => w.sling_user_id === userId && w.date === date && w.start <= start && end <= w.end,
  );
}

/** The computed availability a slot check needs. */
export type AvailabilityLookup = Pick<MonthAvailability, "windows" | "day_ranges">;

export interface SlotAvailability {
  /** The teacher can take the slot as far as blocks/windows go. */
  free: boolean;
  /** Why not: the strongest overlapping block, or "unavailable" when the
   *  slot simply falls outside every available window. null when free. */
  reason: BlockKind | null;
}

/** Can `userId` take the slot? With computed availability the slot must sit
 *  inside one of the teacher's windows (the user-facing definition of
 *  "available"). Blocks alone decide when availability isn't loaded yet, or
 *  when the slot reaches outside the date's computed span — the span is
 *  then out of date (a class was just moved) and says nothing about it. */
export function slotAvailability(
  blocks: AvailabilityBlock[],
  availability: AvailabilityLookup | null | undefined,
  userId: number,
  date: string,
  start: string,
  end: string,
): SlotAvailability {
  const kind = strongestKind(overlappingBlocks(blocks, userId, `${date}T${start}:00`, `${date}T${end}:00`));
  if (kind) return { free: false, reason: kind };
  if (availability) {
    const range = availability.day_ranges.find((r) => r.date === date);
    const covered = range != null && range.open <= start && end <= range.close;
    if (covered && !slotInWindows(availability.windows, userId, date, start, end)) {
      return { free: false, reason: "unavailable" };
    }
  }
  return { free: true, reason: null };
}

/** "12 unavailability blocks, 3 leave days" — the pull/refresh summary. */
export function availabilitySummary(unavailabilityBlocks: number, leaveDays: number): string {
  const blocks = `${unavailabilityBlocks} unavailability block${unavailabilityBlocks === 1 ? "" : "s"}`;
  const days = `${leaveDays} leave day${leaveDays === 1 ? "" : "s"}`;
  return `${blocks}, ${days}`;
}

// ---------- Availability view (teacher × day grid) ----------

export type CellStatus =
  | "available" // free for the whole studio day
  | "partial" // free for part of it
  | "unavailable" // not free at all (unavailability)
  | "pending" // not free / partly free only because of a pending set
  | "leave" // on leave
  | "closed"; // studio closed, no class that day

export interface DayBlock {
  kind: BlockKind;
  recurring: boolean;
  /** Clipped to this date, "HH:MM"; `allDay` when it covers the whole day. */
  start: string;
  end: string;
  allDay: boolean;
}

export interface GridCell {
  date: string;
  status: CellStatus;
  range: DayRange | null;
  windows: AvailabilityWindow[];
  blocks: DayBlock[];
}

export interface GridRow {
  teacher: Teacher;
  cells: GridCell[];
  /** Days with any restriction (not counting closed days). */
  limitedDays: number;
}

/** Every date of "YYYY-MM". */
export function monthDates(month: string): string[] {
  const [y, m] = month.split("-").map(Number);
  const days = new Date(Date.UTC(y, m, 0)).getUTCDate();
  return Array.from({ length: days }, (_, i) => `${month}-${String(i + 1).padStart(2, "0")}`);
}

function nextDate(date: string): string {
  const d = new Date(`${date}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + 1);
  return d.toISOString().slice(0, 10);
}

/** A teacher's blocks on one date, clipped to it, earliest first. */
export function blocksOnDate(blocks: AvailabilityBlock[], userId: number, date: string): DayBlock[] {
  const dayStart = `${date}T00:00:00`;
  const dayEnd = `${nextDate(date)}T00:00:00`;
  return overlappingBlocks(blocks, userId, dayStart, dayEnd)
    .map((b) => {
      const s = wallClock(b.starts_at);
      const e = wallClock(b.ends_at);
      const fromStart = s <= dayStart;
      // 23:59(:59) is how Sling writes "through the end of the day".
      const toEnd = e >= dayEnd || e.slice(0, 16) === `${date}T23:59`;
      return {
        kind: blockKind(b.source),
        recurring: isRecurring(b.source),
        start: fromStart ? "00:00" : s.slice(11, 16),
        end: toEnd ? "24:00" : e.slice(11, 16),
        allDay: fromStart && toEnd,
      };
    })
    .sort((a, b) => a.start.localeCompare(b.start) || a.end.localeCompare(b.end));
}

/** Status of one teacher-day from its windows and blocks. */
export function cellStatus(range: DayRange | null, windows: AvailabilityWindow[], blocks: DayBlock[]): CellStatus {
  if (!range) return "closed";
  const whole = windows.length === 1 && windows[0].start === range.open && windows[0].end === range.close;
  if (whole) return "available";
  // Only blocks that actually reach into the studio day explain a limit.
  const inDay = blocks.filter((b) => b.start < range.close && b.end > range.open);
  if (windows.length === 0) {
    if (inDay.some((b) => b.kind === "leave")) return "leave";
    if (inDay.length > 0 && inDay.every((b) => b.kind === "pending")) return "pending";
    return "unavailable";
  }
  if (inDay.length > 0 && inDay.every((b) => b.kind === "pending")) return "pending";
  return "partial";
}

/** The teacher × day grid for the Availability view (active teachers, A–Z). */
export function buildAvailabilityGrid(
  month: string,
  teachers: Teacher[],
  availability: AvailabilityLookup,
  blocks: AvailabilityBlock[],
): { dates: string[]; rows: GridRow[] } {
  const dates = monthDates(month);
  const rangeByDate = new Map(availability.day_ranges.map((r) => [r.date, r]));
  const windowsByKey = new Map<string, AvailabilityWindow[]>();
  for (const w of availability.windows) {
    const key = `${w.sling_user_id}|${w.date}`;
    const list = windowsByKey.get(key);
    if (list) list.push(w);
    else windowsByKey.set(key, [w]);
  }
  const rows = teachers
    .filter((t) => t.active)
    .sort((a, b) => a.display_name.localeCompare(b.display_name))
    .map((teacher) => {
      const cells = dates.map((date) => {
        const range = rangeByDate.get(date) ?? null;
        const windows = (windowsByKey.get(`${teacher.sling_user_id}|${date}`) ?? [])
          .slice()
          .sort((a, b) => a.start.localeCompare(b.start));
        const dayBlocks = blocksOnDate(blocks, teacher.sling_user_id, date);
        return { date, status: cellStatus(range, windows, dayBlocks), range, windows, blocks: dayBlocks };
      });
      return {
        teacher,
        cells,
        limitedDays: cells.filter((c) => c.status !== "available" && c.status !== "closed").length,
      };
    });
  return { dates, rows };
}

export const CELL_STATUS_LABEL: Record<CellStatus, string> = {
  available: "Available",
  partial: "Partly available",
  unavailable: "Unavailable",
  pending: "Unavailable (pending approval)",
  leave: "On leave",
  closed: "Studio closed",
};

/** "9:45 – 10:45 AM", or "all day". Respects the 12h/24h setting. */
export function describeDayBlock(b: DayBlock, fmt?: TimeFormat): string {
  if (b.allDay) return "all day";
  if (b.start === "00:00") return `until ${formatTime(b.end, fmt)}`;
  if (b.end === "24:00") return `from ${formatTime(b.start, fmt)}`;
  return formatTimeRange(b.start, b.end, fmt);
}

/** One line per fact about a teacher-day, for the detail panel and the
 *  cell tooltip. */
export function describeCell(cell: GridCell, fmt?: TimeFormat): string[] {
  if (!cell.range) return ["Studio closed — no classes this day"];
  const lines = cell.windows.map((w) => `Available ${formatTimeRange(w.start, w.end, fmt)}`);
  if (cell.windows.length === 0) lines.push("Not available this day");
  for (const b of cell.blocks) {
    const label = b.kind === "leave" ? "On leave" : b.kind === "pending" ? "Unavailable (pending approval)" : "Unavailable";
    lines.push(`${label} ${describeDayBlock(b, fmt)}${b.recurring ? " · recurring" : ""}`);
  }
  return lines;
}
