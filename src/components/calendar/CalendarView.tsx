import { useState } from "react";
import type { ProposalDetail, Teacher, ProposalShiftRow, AvailabilityBlock } from "../../types";
import { api } from "../../lib/api";
import type { Issue } from "../../lib/issues";
import { StaleBanner } from "./StaleBanner";
import { IssueQueue } from "./IssueQueue";
import { MonthGrid } from "./MonthGrid";
import { DayEditorPanel } from "./DayEditorPanel";

interface Props {
  proposal: ProposalDetail;
  teachers: Teacher[];
  qualifiedPairs: Set<string>;
  blocks: AvailabilityBlock[];
  issues: Issue[];
  onProposalChanged: () => void;
  onRegenerate: () => void;
  onImportExternal: (slingShiftId: number) => Promise<void>;
  readonly?: boolean;
}

export function CalendarView({
  proposal,
  teachers,
  qualifiedPairs,
  blocks,
  issues,
  onProposalChanged,
  onRegenerate,
  onImportExternal,
  readonly,
}: Props) {
  const [selectedDay, setSelectedDay] = useState<string | null>(null);

  const issueShiftIds = new Set(
    issues.map((w) => w.shift_id).filter((id): id is number => id != null),
  );

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
            onRegenerate={onRegenerate}
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
          qualifiedPairs={qualifiedPairs}
          blocks={blocks}
          warnings={dayWarnings}
          readonly={!!readonly}
          onClose={() => setSelectedDay(null)}
          onAssign={handleAssign}
        />
      )}
    </div>
  );
}
