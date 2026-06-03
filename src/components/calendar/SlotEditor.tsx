import { useState } from "react";
import type { ProposalShiftRow, Teacher } from "../../types";

interface Props {
  shift: ProposalShiftRow;
  teachers: Teacher[];
  qualifiedPairs: Set<string>;
  warnings: string[];
  onSave: (newUserId: number | null) => Promise<void>;
}

export function SlotEditor({ shift, teachers, qualifiedPairs, warnings, onSave }: Props) {
  const [selected, setSelected] = useState<number | null>(shift.sling_user_id);
  const dirty = selected !== shift.sling_user_id;
  const eligible = teachers.filter(
    (t) => t.active && qualifiedPairs.has(`${t.sling_user_id}:${shift.sling_position_id}`),
  );
  return (
    <div className={`bk-slot-editor${dirty ? " bk-dirty" : ""}`}>
      <div className="bk-slot-editor-head">
        <span>{shift.start_time}</span>
        <span>{shift.class_name}</span>
      </div>
      <label>Teacher</label>
      <select
        value={selected ?? ""}
        onChange={(e) => setSelected(e.target.value === "" ? null : Number(e.target.value))}
      >
        <option value="">Unassigned</option>
        {eligible.map((t) => (
          <option key={t.sling_user_id} value={t.sling_user_id}>
            {t.display_name}
          </option>
        ))}
      </select>
      {warnings.length > 0 && (
        <div className="bk-slot-editor-warn">
          {warnings.map((w, i) => <div key={i}>⚠ {w}</div>)}
        </div>
      )}
      {dirty && (
        <button style={{ marginTop: "0.5rem" }} onClick={() => onSave(selected)}>
          Save
        </button>
      )}
    </div>
  );
}
