import type { ProposalShiftRow } from "../../types";
import { SlotRow } from "./SlotRow";

interface Props {
  iso: string;
  inMonth: boolean;
  isToday: boolean;
  isSelected: boolean;
  shifts: ProposalShiftRow[];
  warningShiftIds: Set<number>;
  onSlotClick: (shift: ProposalShiftRow) => void;
}

export function DayCell({
  iso,
  inMonth,
  isToday,
  isSelected,
  shifts,
  warningShiftIds,
  onSlotClick,
}: Props) {
  const dayNum = Number(iso.slice(8, 10));
  const dayLabel = `${weekdayShort(iso)} ${dayNum}`;
  return (
    <div
      className={[
        "bk-day-cell",
        inMonth ? "" : "bk-out-of-month",
        isSelected ? "bk-selected" : "",
      ].filter(Boolean).join(" ")}
    >
      <div className={`bk-day-label${isToday ? " bk-today" : ""}`}>{dayLabel}</div>
      {shifts.map((s) => (
        <SlotRow
          key={s.id}
          shift={s}
          hasWarning={warningShiftIds.has(s.id)}
          onClick={() => onSlotClick(s)}
        />
      ))}
    </div>
  );
}

function weekdayShort(iso: string): string {
  const days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  return days[new Date(iso + "T12:00:00Z").getUTCDay()];
}
