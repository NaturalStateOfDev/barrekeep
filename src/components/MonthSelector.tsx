import { monthWindow } from "../lib/dates";

interface Props {
  today: string;       // "YYYY-MM-DD"
  value: string;       // "YYYY-MM"
  onChange: (next: string) => void;
  /** Optional: label suffix per month (e.g. "(no proposal)") */
  labelFor?: (month: string) => string;
}

const MONTH_NAMES = [
  "January","February","March","April","May","June",
  "July","August","September","October","November","December",
];

function labelMonth(ym: string): string {
  const [y, m] = ym.split("-").map(Number);
  return `${MONTH_NAMES[m - 1]} ${y}`;
}

export function MonthSelector({ today, value, onChange, labelFor }: Props) {
  const months = monthWindow(today);
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
    >
      {months.map((m) => (
        <option key={m} value={m}>
          {labelMonth(m)}{labelFor ? ` ${labelFor(m)}` : ""}
        </option>
      ))}
    </select>
  );
}
