import { useState } from "react";
import { Check } from "lucide-react";
import { api } from "../../lib/api";
import { formatDayShort, formatTimeShort } from "../../lib/dates";
import type { Position, ProposalDetail, ProposedEdit, Teacher } from "../../types";

interface Props {
  edits: ProposedEdit[];
  detail: ProposalDetail;
  positions: Position[];
  teachers: Teacher[];
  onProposalChanged: () => void;
}

/** One row per proposed edit with per-row Apply and Apply-selected — the
 *  fast path the spec calls for while keeping every change reviewable. */
export function EditChecklist({ edits, detail, positions, teachers, onProposalChanged }: Props) {
  const [selected, setSelected] = useState<Set<number>>(
    () => new Set(edits.filter((e) => e.valid).map((_, i) => i).filter((i) => edits[i].valid)),
  );
  const [applied, setApplied] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const slotLabel = (e: ProposedEdit): string => {
    const s = detail.shifts.find((x) => x.id === e.proposal_shift_id);
    if (!s) return `slot #${e.proposal_shift_id}`;
    return `${formatDayShort(s.shift_date)} ${formatTimeShort(s.start_time)} ${s.class_name}`;
  };

  const actionLabel = (e: ProposedEdit): string => {
    if (e.action === "reassign") {
      const t = teachers.find((x) => x.sling_user_id === e.new_user_id);
      return `→ ${t?.display_name ?? `teacher ${e.new_user_id}`}`;
    }
    if (e.action === "change_format") return `format → ${e.new_class_name}`;
    return "unassign (drops the class)";
  };

  const applyOne = async (i: number) => {
    const e = edits[i];
    const reason = `claude: ${e.rationale}`;
    if (e.action === "change_format") {
      const pid = positions.find((p) => p.class_name === e.new_class_name)?.sling_position_id;
      if (!pid) throw new Error(`'${e.new_class_name}' is not a schedulable class`);
      await api.editProposalShiftPosition(e.proposal_shift_id, pid, reason);
    } else {
      await api.editProposalShiftTeacher(
        e.proposal_shift_id,
        e.action === "reassign" ? e.new_user_id ?? null : null,
        reason,
      );
    }
    setApplied((prev) => new Set(prev).add(i));
  };

  const run = async (indexes: number[]) => {
    setBusy(true);
    setError(null);
    try {
      for (const i of indexes) {
        if (!applied.has(i)) await applyOne(i);
      }
      onProposalChanged();
    } catch (err) {
      setError(String(err));
      onProposalChanged();
    } finally {
      setBusy(false);
    }
  };

  const pending = [...selected].filter((i) => !applied.has(i));

  return (
    <div style={{ marginTop: 14 }}>
      <div className="bk-candidate-label">Proposed edits</div>
      {edits.map((e, i) => {
        const isApplied = applied.has(i);
        return (
          <div key={i} className={`bk-edit-row${isApplied ? " applied" : ""}${e.valid ? "" : " invalid"}`}>
            <input
              type="checkbox"
              style={{ accentColor: "var(--accent)" }}
              disabled={!e.valid || isApplied || busy}
              checked={e.valid && !isApplied && selected.has(i)}
              onChange={(ev) => {
                setSelected((prev) => {
                  const next = new Set(prev);
                  if (ev.target.checked) next.add(i);
                  else next.delete(i);
                  return next;
                });
              }}
            />
            <div style={{ minWidth: 0 }}>
              <div>
                <strong>{slotLabel(e)}</strong> <span>{actionLabel(e)}</span>
                {isApplied && <Check size={14} style={{ color: "var(--color-success)", marginLeft: 6, verticalAlign: "-2px" }} />}
              </div>
              <div className="muted" style={{ fontSize: 12 }}>
                {e.valid ? e.rationale : `${e.rationale} — ${e.validation_note ?? "invalid"}`}
              </div>
            </div>
            <button
              className="btn-ghost btn-sm"
              disabled={!e.valid || isApplied || busy}
              onClick={() => run([i])}
            >
              {isApplied ? "Applied" : "Apply"}
            </button>
          </div>
        );
      })}
      <div className="row" style={{ marginTop: 10 }}>
        <button
          className="btn-primary"
          disabled={busy || pending.length === 0}
          onClick={() => run(pending)}
        >
          {busy ? "Applying…" : `Apply selected (${pending.length})`}
        </button>
      </div>
      {error && <div className="error">{error}</div>}
    </div>
  );
}
