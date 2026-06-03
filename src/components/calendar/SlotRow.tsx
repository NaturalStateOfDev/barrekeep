import type { ProposalShiftRow } from "../../types";
import { chipFor } from "../../lib/formatChips";
import { initials } from "../../lib/dates";

function formatTime(hhmm: string): string {
  // hhmm comes in as "HH:MM" 24h. Render compact "5:00a" / "5:30p".
  const [h, m] = hhmm.split(":").map(Number);
  const period = h >= 12 ? "p" : "a";
  const hour12 = h === 0 ? 12 : h > 12 ? h - 12 : h;
  return `${hour12}:${String(m).padStart(2, "0")}${period}`;
}

function teacherLabel(shift: ProposalShiftRow): string {
  if (shift.coteach_label) {
    return shift.coteach_label
      .split("+")
      .map((s) => initials(s.trim()))
      .join("+");
  }
  return initials(shift.teacher_name);
}

interface Props {
  shift: ProposalShiftRow;
  hasWarning: boolean;
  onClick: () => void;
}

export function SlotRow({ shift, hasWarning, onClick }: Props) {
  const chip = chipFor(shift.class_name);
  const unassigned = shift.sling_user_id == null && !shift.coteach_label;
  return (
    <div
      className={`bk-slot-row${shift.is_dropped ? " bk-dropped" : ""}`}
      onClick={onClick}
      role="button"
    >
      <span className="bk-slot-time">{formatTime(shift.start_time)}</span>
      <span
        className="bk-slot-chip"
        style={{
          background: `var(${chip.token})`,
          color: `var(${chip.token}-fg)`,
        }}
        title={shift.class_name}
      >
        {chip.label}
      </span>
      <span className={`bk-slot-teacher${unassigned ? " bk-unassigned" : ""}`}>
        {teacherLabel(shift)}
      </span>
      <span className="bk-slot-warn">{hasWarning ? <span className="bk-warn-dot" /> : null}</span>
    </div>
  );
}
