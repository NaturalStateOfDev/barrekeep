import { CircleCheck, ShieldAlert, X } from "lucide-react";
import type { DraftConflict } from "../../types";
import { CONFLICT_KIND_LABEL } from "../../lib/sync";
import { formatDayShort, formatTimeShort } from "../../lib/dates";

interface Props {
  conflicts: DraftConflict[];
  readonly?: boolean;
  /** Open the day editor on the conflicting slot. */
  onOpenDay: (iso: string) => void;
  onDismiss: () => void;
}

/** Result of re-checking a draft against freshly refreshed availability. */
export function ConflictsPanel({ conflicts, readonly, onOpenDay, onDismiss }: Props) {
  if (conflicts.length === 0) {
    return (
      <div className="bk-conflicts bk-conflicts-clear">
        <CircleCheck size={16} style={{ color: "var(--color-success)", flexShrink: 0 }} />
        <span>Checked against the latest Sling availability — no conflicts. Your draft still fits.</span>
        <button className="btn-ghost btn-sm" onClick={onDismiss} aria-label="dismiss">
          <X size={14} />
        </button>
      </div>
    );
  }
  return (
    <div className="bk-conflicts">
      <div className="bk-conflicts-head">
        <ShieldAlert size={16} style={{ color: "var(--color-danger)", flexShrink: 0 }} />
        <strong>
          {conflicts.length} conflict{conflicts.length === 1 ? "" : "s"} with the latest Sling availability
        </strong>
        <span className="muted">— fix them here before pushing; your other edits are untouched.</span>
        <button className="btn-ghost btn-sm" onClick={onDismiss} aria-label="dismiss" style={{ marginLeft: "auto" }}>
          <X size={14} />
        </button>
      </div>
      <ul className="bk-conflicts-list">
        {conflicts.map((c, i) => (
          <li key={`${c.proposal_shift_id}-${c.kind}-${c.sling_user_id ?? ""}-${i}`}>
            <span className={`bk-conflict-kind bk-conflict-${c.kind}`}>{CONFLICT_KIND_LABEL[c.kind] ?? c.kind}</span>
            <span className="bk-conflict-when">
              {formatDayShort(c.shift_date)} {formatTimeShort(c.start_time)}
            </span>
            <span className="bk-conflict-msg">{c.message}</span>
            <button className="btn-ghost btn-sm" onClick={() => onOpenDay(c.shift_date)} disabled={readonly}>
              Open day
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
