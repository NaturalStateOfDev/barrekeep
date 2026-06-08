import type { Issue } from "../../lib/issues";
import type { Teacher, ProposalShiftRow } from "../../types";

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

  return (
    <div className={`bk-issue-card bk-issue-${severity}`}>
      <div className="bk-issue-header">
        <span className="bk-issue-kind">{labelForKind(issue.kind)}</span>
      </div>
      {slot && (
        <div className="bk-issue-slot">
          <span className="bk-issue-day">{formatDay(slot.shift_date)}</span>
          <span className="bk-issue-time">{formatTime(slot.start_time)}</span>
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
          {suggestedTeacher && <button className="btn-primary" onClick={onApply}>Apply</button>}
          <button className="btn-ghost" onClick={onDismiss}>Dismiss</button>
          {issue.shift_date && <button className="btn-ghost" onClick={onOpenDay}>Open day</button>}
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

// "2026-06-09" → "Tue Jun 9"
function formatDay(iso: string): string {
  const d = new Date(iso + "T00:00:00Z");
  const weekday = ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"][d.getUTCDay()];
  const month = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"][d.getUTCMonth()];
  return `${weekday} ${month} ${d.getUTCDate()}`;
}

// "05:45" → "5:45a"; "13:00" → "1:00p"
function formatTime(hhmm: string): string {
  const [h, m] = hhmm.split(":").map(Number);
  const period = h >= 12 ? "p" : "a";
  const hour12 = h === 0 ? 12 : h > 12 ? h - 12 : h;
  return `${hour12}:${String(m).padStart(2, "0")}${period}`;
}
