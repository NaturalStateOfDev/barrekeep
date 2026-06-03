import type { ProposalShiftRow, Teacher } from "../../types";
import type { Issue } from "../../lib/issues";
import { SlotEditor } from "./SlotEditor";

interface Props {
  iso: string;
  shifts: ProposalShiftRow[];
  teachers: Teacher[];
  qualifiedPairs: Set<string>;
  warnings: Issue[];
  onClose: () => void;
  onSave: (proposalShiftId: number, newUserId: number | null) => Promise<void>;
}

export function DayPanel({
  iso,
  shifts,
  teachers,
  qualifiedPairs,
  warnings,
  onClose,
  onSave,
}: Props) {
  return (
    <aside className="bk-day-panel" aria-label={`Schedule for ${iso}`}>
      <div className="bk-day-panel-head">
        <strong>{prettyDate(iso)}</strong>
        <button onClick={onClose}>Close</button>
      </div>
      <div className="bk-day-panel-body">
        {shifts.length === 0 && <em>No classes scheduled.</em>}
        {shifts.map((s) => {
          const slotWarnings = warnings
            .filter((w) => w.shift_id === s.id)
            .map((w) => w.message);
          return (
            <SlotEditor
              key={s.id}
              shift={s}
              teachers={teachers}
              qualifiedPairs={qualifiedPairs}
              warnings={slotWarnings}
              onSave={(newUserId) => onSave(s.id, newUserId)}
            />
          );
        })}
      </div>
    </aside>
  );
}

function prettyDate(iso: string): string {
  const d = new Date(iso + "T12:00:00Z");
  const days = ["Sunday","Monday","Tuesday","Wednesday","Thursday","Friday","Saturday"];
  const months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
  return `${days[d.getUTCDay()]} ${months[d.getUTCMonth()]} ${d.getUTCDate()}, ${d.getUTCFullYear()}`;
}
