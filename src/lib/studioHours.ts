// Studio hours form helpers (Settings → Studio hours). weekday: 0 = Monday …
// 6 = Sunday, matching `studio_hours.weekday` in the database.

import type { DayHours } from "../types";

export const WEEKDAY_LABELS = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];

/** Seven closed days — the form's starting point. */
export function emptyWeek(): DayHours[] {
  return WEEKDAY_LABELS.map((_, weekday) => ({ weekday, closed: true, open: null, close: null }));
}

/** Exactly one entry per weekday, Monday first; missing days are closed. */
export function normalizeWeek(days: DayHours[]): DayHours[] {
  return emptyWeek().map((blank) => {
    const d = days.find((x) => x.weekday === blank.weekday);
    if (!d || d.closed) return blank;
    return { weekday: d.weekday, closed: false, open: d.open, close: d.close };
  });
}

/** Why the week can't be saved, or null when it's valid. */
export function hoursError(days: DayHours[]): string | null {
  for (const d of days) {
    if (d.closed) continue;
    const name = WEEKDAY_LABELS[d.weekday] ?? `day ${d.weekday}`;
    if (!d.open || !d.close) return `${name}: set both an opening and a closing time, or mark it closed.`;
    if (d.open >= d.close) return `${name}: the opening time must be before the closing time.`;
  }
  return null;
}
