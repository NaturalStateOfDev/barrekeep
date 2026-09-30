export interface MonthGridCell {
  iso: string;    // YYYY-MM-DD
  inMonth: boolean;
}

// targetMonth: "YYYY-MM"
export function buildMonthGrid(targetMonth: string): MonthGridCell[][] {
  const [yearStr, monthStr] = targetMonth.split("-");
  const year = Number(yearStr);
  const month = Number(monthStr); // 1-12

  // Day-of-week of the 1st (0 = Sunday)
  const firstDow = new Date(Date.UTC(year, month - 1, 1)).getUTCDay();
  // First Sunday on or before the 1st
  const gridStart = new Date(Date.UTC(year, month - 1, 1 - firstDow));

  const rows: MonthGridCell[][] = [];
  for (let w = 0; w < 6; w++) {
    const row: MonthGridCell[] = [];
    for (let d = 0; d < 7; d++) {
      const dt = new Date(gridStart);
      dt.setUTCDate(gridStart.getUTCDate() + w * 7 + d);
      row.push({
        iso: dt.toISOString().slice(0, 10),
        inMonth: dt.getUTCFullYear() === year && dt.getUTCMonth() === month - 1,
      });
    }
    rows.push(row);
  }
  return rows;
}

// ISO 8601 week: Mon-Sun, week 1 contains the first Thursday of the year.
export function isoWeekKey(iso: string): string {
  const d = new Date(iso + "T00:00:00Z");
  // Set to Thursday of this week (ISO weeks are anchored on Thursday)
  const day = d.getUTCDay() || 7; // Sunday becomes 7
  d.setUTCDate(d.getUTCDate() + 4 - day);
  const year = d.getUTCFullYear();
  const jan1 = new Date(Date.UTC(year, 0, 1));
  const week = Math.ceil((((d.getTime() - jan1.getTime()) / 86400000) + 1) / 7);
  return `${year}-W${String(week).padStart(2, "0")}`;
}

export function initials(displayName: string | null): string {
  if (!displayName) return "??";
  const parts = displayName.trim().split(/\s+/);
  if (parts.length === 1) return parts[0][0].toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

// today: ISO date "YYYY-MM-DD"
// returns four "YYYY-MM" strings: prev, current, next, next+1
export function monthWindow(today: string): string[] {
  const [y, m] = today.split("-").map(Number);
  const out: string[] = [];
  for (const delta of [-1, 0, 1, 2]) {
    let ny = y;
    let nm = m + delta;
    while (nm < 1) { nm += 12; ny -= 1; }
    while (nm > 12) { nm -= 12; ny += 1; }
    out.push(`${ny}-${String(nm).padStart(2, "0")}`);
  }
  return out;
}

// targetMonth: "YYYY-MM"; today: "YYYY-MM-DD"
export function isReadOnlyMonth(targetMonth: string, today: string): boolean {
  return targetMonth < today.slice(0, 7);
}

export const WEEKDAYS_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

export const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

// "2026-08" → "August 2026"
export function monthLabel(ym: string): string {
  const [y, m] = ym.split("-").map(Number);
  return `${MONTH_NAMES[m - 1]} ${y}`;
}

// ---- Time display ------------------------------------------------------
// Stored values, API payloads, algorithm rules, Sling requests and Claude
// prompts all use 24-hour "HH:MM". These helpers are DISPLAY-ONLY; the user's
// choice (Settings → Display, app_settings.time_format) reaches components via
// useTimeFormat() in ./timeFormat.

export type TimeFormat = "12h" | "24h";
export const DEFAULT_TIME_FORMAT: TimeFormat = "12h";

/** Parse "HH:MM" or "HH:MM:SS"; null when it isn't a time. */
function parseHhmm(hhmm: string): [number, number] | null {
  const m = /^(\d{1,2}):(\d{2})(?::\d{2}(?:\.\d+)?)?$/.exec((hhmm ?? "").trim());
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  if (h > 23 || min > 59) return null;
  return [h, min];
}

function parts12(h: number): [number, "AM" | "PM"] {
  return [h % 12 === 0 ? 12 : h % 12, h >= 12 ? "PM" : "AM"];
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** "09:45" → "9:45 AM" (12h) or "09:45" (24h). Unparseable input is returned unchanged. */
export function formatTime(hhmm: string, fmt: TimeFormat = DEFAULT_TIME_FORMAT): string {
  const t = parseHhmm(hhmm);
  if (!t) return hhmm;
  const [h, m] = t;
  if (fmt === "24h") return `${pad2(h)}:${pad2(m)}`;
  const [h12, period] = parts12(h);
  return `${h12}:${pad2(m)} ${period}`;
}

/** Compact form for tight spaces: "05:45" → "5:45a" (12h) or "05:45" (24h). */
export function formatTimeShort(hhmm: string, fmt: TimeFormat = DEFAULT_TIME_FORMAT): string {
  const t = parseHhmm(hhmm);
  if (!t) return hhmm;
  const [h, m] = t;
  if (fmt === "24h") return `${pad2(h)}:${pad2(m)}`;
  const [h12, period] = parts12(h);
  return `${h12}:${pad2(m)}${period === "AM" ? "a" : "p"}`;
}

/** "09:45","10:35" → "9:45 – 10:35 AM"; across noon "11:30 AM – 12:20 PM";
 *  24h "09:45–10:35". `compact` uses the short forms ("9:45a–10:35a"). */
export function formatTimeRange(
  start: string,
  end: string,
  fmt: TimeFormat = DEFAULT_TIME_FORMAT,
  compact = false,
): string {
  if (fmt === "24h" || compact) return `${formatTimeShort(start, fmt)}–${formatTimeShort(end, fmt)}`;
  const a = parseHhmm(start);
  const b = parseHhmm(end);
  if (a && b && parts12(a[0])[1] === parts12(b[0])[1]) {
    const [h12] = parts12(a[0]);
    return `${h12}:${pad2(a[1])} – ${formatTime(end, fmt)}`;
  }
  return `${formatTime(start, fmt)} – ${formatTime(end, fmt)}`;
}

/** Reformat every "HH:MM" inside an app-generated message (e.g. the
 *  backend's conflict text "Alex is marked unavailable (05:00–06:00)").
 *  Not for Claude's free text. 24h returns the text unchanged. */
export function formatTimesInText(text: string, fmt: TimeFormat = DEFAULT_TIME_FORMAT): string {
  if (fmt === "24h") return text;
  return text.replace(/(?<![\d:])([01]?\d|2[0-3]):([0-5]\d)(?![\d:])/g, (m) => formatTime(m, fmt));
}

/** "Mon 09:45" (weekday + time slot label) → "Mon 9:45 AM". */
export function formatSlotLabel(label: string, fmt: TimeFormat = DEFAULT_TIME_FORMAT): string {
  const i = label.lastIndexOf(" ");
  return i < 0 ? label : `${label.slice(0, i)} ${formatTime(label.slice(i + 1), fmt)}`;
}

// "2026-06-09" → "Tue Jun 9"
export function formatDayShort(iso: string): string {
  const d = new Date(iso + "T00:00:00Z");
  const month = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"][d.getUTCMonth()];
  return `${WEEKDAYS_SHORT[d.getUTCDay()]} ${month} ${d.getUTCDate()}`;
}

// "2026-08-12" → "Wednesday, August 12"
export function prettyDayLong(iso: string): string {
  const d = new Date(iso + "T12:00:00Z");
  const days = ["Sunday","Monday","Tuesday","Wednesday","Thursday","Friday","Saturday"];
  return `${days[d.getUTCDay()]}, ${MONTH_NAMES[d.getUTCMonth()]} ${d.getUTCDate()}`;
}

/** The studio's time zone (matches `sling::STUDIO_TZ` in Rust). */
export const STUDIO_TZ = "America/Chicago";

// An offset (or Z) right after a time of day — not the "-20" of a bare date.
const OFFSET_RE = /\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(Z|[+-]\d{2}(?::?\d{2})?)$/;

/** Parse a backend timestamp into a Date. The backend sends TIMESTAMPTZ
 *  values as ISO-8601 UTC ("2026-11-01T06:30:00Z"); older / other forms
 *  ("2026-07-03 09:20:44+00", "…-05", "…+00:00") are accepted too. A string
 *  without an offset is a local wall time. null when unparseable. */
export function parseTimestamp(ts: string): Date | null {
  let s = (ts ?? "").trim().replace(" ", "T");
  // "+00" / "-05" → "+00:00" / "-05:00" (Date only accepts full offsets).
  if (OFFSET_RE.test(s)) {
    s = s.replace(/([+-]\d{2})$/, "$1:00").replace(/([+-]\d{2})(\d{2})$/, "$1:$2");
  }
  const d = new Date(s);
  return Number.isNaN(d.getTime()) ? null : d;
}

/** Wall-clock parts of `d` in `timeZone` (undefined = the viewer's local zone). */
function zonedParts(d: Date, timeZone?: string) {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  }).formatToParts(d);
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "00";
  return {
    date: `${get("year")}-${get("month")}-${get("day")}`,
    hh: get("hour"),
    mm: get("minute"),
    ss: get("second"),
  };
}

/** A backend timestamp shown in local time: 24h → "2026-07-03 09:20:44";
 *  12h → "2026-07-03 9:20 AM". Unparseable input is returned unchanged.
 *  `timeZone` is for tests; the app shows the viewer's local zone. */
export function formatTimestamp(
  ts: string,
  fmt: TimeFormat = DEFAULT_TIME_FORMAT,
  timeZone?: string,
): string {
  const d = parseTimestamp(ts);
  if (!d) return ts;
  const p = zonedParts(d, timeZone);
  if (fmt === "24h") return `${p.date} ${p.hh}:${p.mm}:${p.ss}`;
  return `${p.date} ${formatTime(`${p.hh}:${p.mm}`, fmt)}`;
}

/** Local calendar date "YYYY-MM-DD" of a backend timestamp. */
export function formatLocalDate(ts: string, timeZone?: string): string {
  const d = parseTimestamp(ts);
  return d ? zonedParts(d, timeZone).date : ts;
}

// Normalize a timestamp to comparable studio wall-clock form 'YYYY-MM-DDTHH:MM:SS'.
// Availability blocks arrive as instants (ISO UTC 'Z', or any offset); shift
// times are studio-local strings 'YYYY-MM-DDTHH:MM:SS' with no offset. An
// instant is converted to studio time (America/Chicago, DST-aware); a string
// without an offset is already studio-local and only trimmed. Both sides
// must be normalized before comparing lexicographically.
export function wallClock(ts: string): string {
  const s = ts.trim().replace(" ", "T");
  if (OFFSET_RE.test(s)) {
    const d = parseTimestamp(s);
    if (d) {
      const p = zonedParts(d, STUDIO_TZ);
      return `${p.date}T${p.hh}:${p.mm}:${p.ss}`;
    }
  }
  return s.replace(/\.\d+$/, "").slice(0, 19);
}
