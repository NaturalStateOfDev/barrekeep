import { useState } from "react";
import { X, UserRoundCog } from "lucide-react";
import type { ProposalShiftRow, Teacher, AvailabilityBlock } from "../../types";
import type { Issue } from "../../lib/issues";
import { candidatesFor } from "../../lib/candidates";
import { formatTimeShort, prettyDayLong } from "../../lib/dates";
import { Avatar } from "../ui/Avatar";
import { ClassChip } from "../ui/ClassChip";

interface Props {
  iso: string;
  shifts: ProposalShiftRow[];
  allShifts: ProposalShiftRow[];
  teachers: Teacher[];
  qualifiedPairs: Set<string>;
  blocks: AvailabilityBlock[];
  warnings: Issue[];
  readonly: boolean;
  onClose: () => void;
  onAssign: (proposalShiftId: number, newUserId: number | null) => Promise<void>;
}

/** Slide-in day editor: every class that day, with a candidate list to
 *  (re)assign a teacher. Candidates show qualification / leave / cap notes. */
export function DayEditorPanel({
  iso,
  shifts,
  allShifts,
  teachers,
  qualifiedPairs,
  blocks,
  warnings,
  readonly,
  onClose,
  onAssign,
}: Props) {
  const [editing, setEditing] = useState<number | null>(null);
  const [saving, setSaving] = useState(false);

  const assign = async (shiftId: number, userId: number | null) => {
    setSaving(true);
    try {
      await onAssign(shiftId, userId);
      setEditing(null);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="bk-day-overlay">
      <div className="bk-day-backdrop" onClick={onClose} />
      <aside className="bk-day-panel" aria-label={`Schedule for ${iso}`}>
        <div className="bk-day-panel-head">
          <div>
            <div className="bk-day-panel-title">{prettyDayLong(iso)}</div>
            <div className="bk-day-panel-sub">
              {shifts.length} class{shifts.length === 1 ? "" : "es"}
            </div>
          </div>
          <button className="bk-day-panel-close" onClick={onClose} aria-label="Close">
            <X size={20} />
          </button>
        </div>
        <div className="bk-day-panel-body">
          {shifts.length === 0 && (
            <div className="muted" style={{ padding: "24px 6px", textAlign: "center" }}>
              No classes scheduled this day.
            </div>
          )}
          {shifts.map((s) => {
            const slotWarnings = warnings.filter((w) => w.shift_id === s.id);
            const unassigned = s.sling_user_id == null && !s.coteach_label;
            return (
              <div key={s.id} className="bk-day-slot">
                <div className="bk-day-slot-head">
                  <span className="bk-day-slot-time">{formatTimeShort(s.start_time)}</span>
                  <ClassChip className={s.class_name} size="md" />
                  <span className={`bk-day-slot-teacher${unassigned ? " bk-unassigned" : ""}`}>
                    {s.coteach_label ? (
                      <>
                        {s.coteach_label.split("+").map((part) => (
                          <Avatar key={part} name={part.trim()} size={22} />
                        ))}
                        <span>{s.coteach_label}</span>
                      </>
                    ) : unassigned ? (
                      "Unassigned"
                    ) : (
                      <>
                        <Avatar name={s.teacher_name} size={22} />
                        <span>{s.teacher_name}</span>
                      </>
                    )}
                  </span>
                </div>
                {slotWarnings.length > 0 && (
                  <div className="bk-day-slot-warn">
                    {slotWarnings.map((w, i) => (
                      <div key={i}>⚠ {w.message}</div>
                    ))}
                  </div>
                )}
                {!readonly && !s.is_dropped && (
                  <div style={{ marginTop: 10 }}>
                    {editing === s.id ? (
                      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                        <div className="bk-candidate-label">Assign teacher</div>
                        {candidatesFor(s, allShifts, teachers, qualifiedPairs, blocks).map((c) => (
                          <button
                            key={c.teacher.sling_user_id}
                            className={`bk-candidate${c.current ? " bk-current" : ""}`}
                            disabled={!c.qualified || saving}
                            onClick={() => assign(s.id, c.teacher.sling_user_id)}
                          >
                            <Avatar name={c.teacher.display_name} size={22} />
                            <span>{c.teacher.display_name}</span>
                            {c.note ? (
                              <span className={`note${c.note === "not qualified" ? " bk-danger" : ""}`}>
                                {c.note}
                              </span>
                            ) : c.current ? (
                              <span className="note bk-quiet">current</span>
                            ) : null}
                          </button>
                        ))}
                        {s.sling_user_id != null && (
                          <button
                            className="bk-candidate"
                            disabled={saving}
                            onClick={() => assign(s.id, null)}
                          >
                            <span className="muted">Mark unassigned</span>
                          </button>
                        )}
                        <button className="bk-candidate-cancel" onClick={() => setEditing(null)}>
                          Cancel
                        </button>
                      </div>
                    ) : (
                      <button className="btn-ghost btn-sm" onClick={() => setEditing(s.id)}>
                        <UserRoundCog size={14} /> {unassigned ? "Assign" : "Change teacher"}
                      </button>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </aside>
    </div>
  );
}
