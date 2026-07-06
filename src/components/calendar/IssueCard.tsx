import type { Issue } from "../../lib/issues";
import type { Teacher, ProposalShiftRow } from "../../types";
import { formatDayShort, formatTimeShort } from "../../lib/dates";

interface Props {
  issue: Issue;
  slot?: ProposalShiftRow | null;
  suggestedTeacher?: Teacher | null;
  readonly: boolean;
  onApply: () => void;
  onDismiss: () => void;
  onOpenDay: () => void;
}

const SEVERITY: Record<Issue["kind"], string> = {
  unassigned: "danger",
  leave_conflict: "danger",
  teacher_deactivated: "danger",
  qualification: "warn",
  over_cap: "info",
  external_shift: "info",
  new_teacher: "info",
};

export function IssueCard({ issue, slot, suggestedTeacher, readonly, onApply, onDismiss, onOpenDay }: Props) {
  const severity = SEVERITY[issue.kind];
  const canApply = suggestedTeacher != null || issue.kind === "external_shift";

  return (
    <div className={`bk-issue-card bk-issue-${severity}`}>
      <div className="bk-issue-header">
        <span>{labelForKind(issue.kind)}</span>
      </div>
      {slot && (
        <div className="bk-issue-slot">
          <span className="bk-issue-day">{formatDayShort(slot.shift_date)}</span>
          <span className="bk-issue-time">{formatTimeShort(slot.start_time)}</span>
          <span className="bk-issue-class">{slot.class_name}</span>
        </div>
      )}
      <div className="bk-issue-msg">{issue.message}</div>
      {suggestedTeacher && (
        <div className="bk-issue-fix">
          Suggest: <strong>{suggestedTeacher.display_name}</strong>
        </div>
      )}
      {!readonly && (
        <div className="bk-issue-actions">
          {canApply && (
            <button className="btn-primary btn-sm" onClick={onApply}>
              {issue.kind === "external_shift" ? "Import" : "Apply"}
            </button>
          )}
          <button className="btn-ghost btn-sm" onClick={onDismiss}>Dismiss</button>
          {issue.shift_date && (
            <button className="btn-ghost btn-sm" onClick={onOpenDay}>Open day</button>
          )}
        </div>
      )}
    </div>
  );
}

function labelForKind(k: Issue["kind"]): string {
  switch (k) {
    case "unassigned": return "Unassigned";
    case "over_cap": return "Over cap";
    case "qualification": return "Not qualified";
    case "leave_conflict": return "Leave conflict";
    case "teacher_deactivated": return "Deactivated";
    case "external_shift": return "External shift";
    case "new_teacher": return "New teacher";
  }
}
