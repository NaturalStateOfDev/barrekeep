import type { ProposalShiftRow } from "../../types";
import { formatTimeShort } from "../../lib/dates";
import { Avatar } from "../ui/Avatar";
import { ClassChip } from "../ui/ClassChip";

interface Props {
  shift: ProposalShiftRow;
  hasWarning: boolean;
  onClick: () => void;
}

export function SlotRow({ shift, hasWarning, onClick }: Props) {
  const unassigned = shift.sling_user_id == null && !shift.coteach_label;
  return (
    <div
      className={`bk-slot-row${shift.is_dropped ? " bk-dropped" : ""}`}
      onClick={onClick}
      role="button"
    >
      <span className="bk-slot-time">{formatTimeShort(shift.start_time)}</span>
      <ClassChip className={shift.class_name} />
      <span style={{ display: "inline-flex", gap: 2 }}>
        {unassigned ? (
          <span className="bk-slot-unassigned">!</span>
        ) : shift.coteach_label ? (
          shift.coteach_label
            .split("+")
            .map((part) => <Avatar key={part} name={part.trim()} size={18} />)
        ) : (
          <Avatar name={shift.teacher_name} size={18} />
        )}
      </span>
      <span>{hasWarning ? <span className="bk-warn-dot" /> : null}</span>
    </div>
  );
}
