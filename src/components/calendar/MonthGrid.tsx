import type { ProposalShiftRow } from "../../types";
import { buildMonthGrid } from "../../lib/dates";
import { DayCell } from "./DayCell";

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

interface Props {
  targetMonth: string; // "YYYY-MM"
  shifts: ProposalShiftRow[];
  warningShiftIds: Set<number>;
  selectedDay: string | null;
  todayIso: string;
  onDayClick: (iso: string) => void;
  onSlotClick: (shift: ProposalShiftRow) => void;
}

export function MonthGrid({
  targetMonth,
  shifts,
  warningShiftIds,
  selectedDay,
  todayIso,
  onDayClick,
  onSlotClick,
}: Props) {
  const grid = buildMonthGrid(targetMonth);
  const byDate = new Map<string, ProposalShiftRow[]>();
  for (const s of shifts) {
    const list = byDate.get(s.shift_date) ?? [];
    list.push(s);
    byDate.set(s.shift_date, list);
  }
  return (
    <>
      <div className="bk-month-grid-head">
        {WEEKDAYS.map((w) => <div key={w}>{w}</div>)}
      </div>
      <div className="bk-month-grid">
        {grid.flat().map((cell) => (
          <div key={cell.iso} onClick={() => cell.inMonth && onDayClick(cell.iso)}>
            <DayCell
              iso={cell.iso}
              inMonth={cell.inMonth}
              isToday={cell.iso === todayIso}
              isSelected={cell.iso === selectedDay}
              shifts={byDate.get(cell.iso) ?? []}
              warningShiftIds={warningShiftIds}
              onSlotClick={onSlotClick}
            />
          </div>
        ))}
      </div>
    </>
  );
}
