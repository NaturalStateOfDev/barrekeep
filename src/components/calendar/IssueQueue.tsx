import { useState } from "react";
import type { Issue } from "../../lib/issues";
import type { Teacher, ProposalShiftRow, AvailabilityBlock, NewUserSummary } from "../../types";
import { suggestSwap } from "../../lib/suggestFix";
import { IssueCard } from "./IssueCard";

interface Props {
  issues: Issue[];
  shifts: ProposalShiftRow[];
  teachers: Teacher[];
  qualifiedPairs: Set<string>;
  blocks: AvailabilityBlock[];
  newUsers: NewUserSummary[];
  readonly: boolean;
  onApplySwap: (proposalShiftId: number, newUserId: number) => Promise<void>;
  onImportExternal: (slingShiftId: number) => Promise<void>;
  onAddTeacher: (input: { sling_user_id: number; display_name: string;
    weekly_target: number; weekly_max: number; is_lead: boolean }) => Promise<void>;
  onOpenDay: (iso: string) => void;
}

export function IssueQueue({
  issues, shifts, teachers, qualifiedPairs, blocks, newUsers,
  readonly, onApplySwap, onImportExternal, onAddTeacher, onOpenDay,
}: Props) {
  const [dismissed, setDismissed] = useState<Set<string>>(new Set());

  const key = (i: Issue) => `${i.kind}|${i.shift_id ?? ""}|${i.ref ?? ""}`;
  const visible = issues.filter((i) => !dismissed.has(key(i)));

  return (
    <aside className="bk-issue-queue" aria-label="schedule issues">
      <div className="bk-issue-queue-head">
        Issues ({visible.length})
      </div>
      <div className="bk-issue-queue-body">
        {visible.length === 0 && <div className="muted">No issues.</div>}
        {visible.map((issue, i) => {
          const k = key(issue);
          const slot = issue.shift_id != null
            ? shifts.find((s) => s.id === issue.shift_id)
            : null;
          const suggested = slot
            ? suggestSwap(slot, shifts, teachers, qualifiedPairs, blocks)
            : null;
          const handleApply = async () => {
            if (issue.kind === "external_shift" && typeof issue.ref === "number") {
              await onImportExternal(issue.ref);
            } else if (issue.kind === "new_teacher") {
              // Wired via onApplyForm below — top-level Apply button is unused for new_teacher
            } else if (slot && suggested) {
              await onApplySwap(slot.id, suggested.sling_user_id);
            }
          };

          const handleApplyForm = async (params: { target: number; max: number; lead: boolean }) => {
            if (issue.kind !== "new_teacher" || typeof issue.ref !== "number") return;
            const u = newUsers.find((x) => x.sling_user_id === issue.ref);
            if (!u) return;
            await onAddTeacher({
              sling_user_id: u.sling_user_id,
              display_name: u.display_name,
              weekly_target: params.target,
              weekly_max: params.max,
              is_lead: params.lead,
            });
          };

          return (
            <IssueCard
              key={`${k}-${i}`}
              issue={issue}
              slot={slot}
              suggestedTeacher={suggested}
              readonly={readonly}
              onApply={handleApply}
              onDismiss={() => setDismissed((s) => new Set(s).add(k))}
              onOpenDay={() => issue.shift_date && onOpenDay(issue.shift_date)}
              onApplyForm={handleApplyForm}
            />
          );
        })}
      </div>
    </aside>
  );
}
