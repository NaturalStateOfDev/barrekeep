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

// "05:45" → "5:45a"; "13:00" → "1:00p"
export function formatTimeShort(hhmm: string): string {
  const [h, m] = hhmm.split(":").map(Number);
  const period = h >= 12 ? "p" : "a";
  const hour12 = h === 0 ? 12 : h > 12 ? h - 12 : h;
  return `${hour12}:${String(m).padStart(2, "0")}${period}`;
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

// DuckDB returns 'YYYY-MM-DD HH:MM:SS+TZ'. Trim to local-ish display.
export function formatTimestamp(iso: string): string {
  return iso.replace("T", " ").replace(/\.\d+/, "").slice(0, 19);
}

// Normalize a timestamp to comparable local wall-clock form 'YYYY-MM-DDTHH:MM:SS'.
// DuckDB TIMESTAMPTZ casts render as 'YYYY-MM-DD HH:MM:SS[.frac]±TZ' (space
// separator + offset, in the laptop's local zone = studio time); shift-local
// strings are built as 'YYYY-MM-DDTHH:MM:SS'. Lexicographic comparison across
// the two formats breaks at the separator (' ' < 'T'), so both sides must be
// normalized before comparing.
export function wallClock(ts: string): string {
  return ts
    .replace(" ", "T")
    .replace(/(\.\d+)?([+-]\d{2}(:?\d{2})?)?$/, "")
    .slice(0, 19);
}
