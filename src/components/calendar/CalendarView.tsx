import { useState } from "react";
import type {
  ProposalDetail,
  Teacher,
  Position,
  ProposalShiftRow,
  AvailabilityBlock,
  DraftConflict,
} from "../../types";
import type { AvailabilityLookup } from "../../lib/availability";
import { api } from "../../lib/api";
import type { Issue } from "../../lib/issues";
import { StaleBanner } from "./StaleBanner";
import { ConflictsPanel } from "./ConflictsPanel";
import { IssueQueue } from "./IssueQueue";
import { MonthGrid } from "./MonthGrid";
import { DayEditorPanel } from "./DayEditorPanel";

interface Props {
  proposal: ProposalDetail;
  teachers: Teacher[];
  positions: Position[];
  qualifiedPairs: Set<string>;
  blocks: AvailabilityBlock[];
  /** Computed availability for the month (null until loaded). */
  availability: AvailabilityLookup | null;
  issues: Issue[];
  onProposalChanged: () => void;
  onRegenerate: () => void;
  /** Re-pull availability for current/future months and re-check this draft. */
  onRefreshAvailability: () => void;
  refreshing?: boolean;
  /** Result of the last availability re-check (null = none shown). */
  conflicts: DraftConflict[] | null;
  onDismissConflicts: () => void;
  onImportExternal: (slingShiftId: number) => Promise<void>;
  readonly?: boolean;
}

export function CalendarView({
  proposal,
  teachers,
  positions,
  qualifiedPairs,
  blocks,
  availability,
  issues,
  onProposalChanged,
  onRegenerate,
  onRefreshAvailability,
  refreshing,
  conflicts,
  onDismissConflicts,
  onImportExternal,
  readonly,
}: Props) {
  const [selectedDay, setSelectedDay] = useState<string | null>(null);

  const issueShiftIds = new Set(
    issues.map((w) => w.shift_id).filter((id): id is number => id != null),
  );
  for (const c of conflicts ?? []) issueShiftIds.add(c.proposal_shift_id);

  const todayIso = new Date().toISOString().slice(0, 10);
  const targetMonth = proposal.summary.target_month.slice(0, 7);

  const dayShifts = selectedDay
    ? proposal.shifts.filter((s) => s.shift_date === selectedDay)
    : [];
  const dayWarnings = selectedDay
    ? issues.filter((w) => w.shift_date === selectedDay)
    : [];

  const handleAssign = async (proposalShiftId: number, newUserId: number | null) => {
    if (readonly) return;
    await api.editProposalShiftTeacher(proposalShiftId, newUserId, null);
    onProposalChanged();
  };

  const handleChangeFormat = async (proposalShiftId: number, newPositionId: number) => {
    if (readonly) return;
    await api.editProposalShiftPosition(proposalShiftId, newPositionId, null);
    onProposalChanged();
  };

  const handleSlotClick = (shift: ProposalShiftRow) => {
    if (readonly) return;
    setSelectedDay(shift.shift_date);
  };
  const handleDayClick = (iso: string) => {
    if (readonly) return;
    setSelectedDay(iso);
  };

  return (
    <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr) 316px", gap: 16, alignItems: "start" }}>
      <div style={{ minWidth: 0 }}>
        {proposal.is_stale && proposal.last_pulled_at && (
          <StaleBanner
            lastPulledAt={proposal.last_pulled_at}
            generatedAt={proposal.summary.generated_at}
            refreshing={refreshing}
            readonly={readonly}
            onRefreshAvailability={onRefreshAvailability}
            onRegenerate={onRegenerate}
          />
        )}
        {conflicts && (
          <ConflictsPanel
            conflicts={conflicts}
            readonly={readonly}
            onOpenDay={(iso) => setSelectedDay(iso)}
            onDismiss={onDismissConflicts}
          />
        )}
        <MonthGrid
          targetMonth={targetMonth}
          shifts={proposal.shifts}
          warningShiftIds={issueShiftIds}
          selectedDay={selectedDay}
          todayIso={todayIso}
          onDayClick={handleDayClick}
          onSlotClick={handleSlotClick}
        />
      </div>
      <IssueQueue
        issues={issues}
        shifts={proposal.shifts}
        teachers={teachers}
        qualifiedPairs={qualifiedPairs}
        blocks={blocks}
        availability={availability}
        readonly={!!readonly}
        onApplySwap={async (shiftId, userId) => {
          await handleAssign(shiftId, userId);
        }}
        onImportExternal={onImportExternal}
        onOpenDay={(iso) => setSelectedDay(iso)}
      />
      {selectedDay && (
        <DayEditorPanel
          iso={selectedDay}
          shifts={dayShifts}
          allShifts={proposal.shifts}
          teachers={teachers}
          positions={positions}
          qualifiedPairs={qualifiedPairs}
          blocks={blocks}
          availability={availability}
          warnings={dayWarnings}
          readonly={!!readonly}
          onClose={() => setSelectedDay(null)}
          onAssign={handleAssign}
          onChangeFormat={handleChangeFormat}
        />
      )}
    </div>
  );
}
