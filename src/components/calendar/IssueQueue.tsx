import { useState } from "react";
import type { Issue } from "../../lib/issues";
import type { Teacher, ProposalShiftRow, AvailabilityBlock } from "../../types";
import { suggestSwap } from "../../lib/suggestFix";
import { IssueCard } from "./IssueCard";

interface Props {
  issues: Issue[];
  shifts: ProposalShiftRow[];
  teachers: Teacher[];
  qualifiedPairs: Set<string>;
  blocks: AvailabilityBlock[];
  readonly: boolean;
  onApplySwap: (proposalShiftId: number, newUserId: number) => Promise<void>;
  onImportExternal: (slingShiftId: number) => Promise<void>;
  onOpenDay: (iso: string) => void;
}

export function IssueQueue({
  issues, shifts, teachers, qualifiedPairs, blocks,
  readonly, onApplySwap, onImportExternal, onOpenDay,
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
            } else if (slot && suggested) {
              await onApplySwap(slot.id, suggested.sling_user_id);
            }
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
            />
          );
        })}
      </div>
    </aside>
  );
}
