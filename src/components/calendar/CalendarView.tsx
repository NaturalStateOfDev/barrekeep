import { useEffect, useMemo, useState } from "react";
import type { ProposalDetail, Teacher, ProposalShiftRow, AvailabilityBlock, ExternalShiftRow } from "../../types";
import { api } from "../../lib/api";
import { computeIssues, type Issue } from "../../lib/issues";
import { CalendarHeader } from "./CalendarHeader";
import { StaleBanner } from "./StaleBanner";
import { IssueQueue } from "./IssueQueue";
import { MonthGrid } from "./MonthGrid";
import { DayPanel } from "./DayPanel";

interface Props {
  proposal: ProposalDetail;
  onProposalChanged: () => void;
  onRegenerate: () => void;
  readonly?: boolean;
}

export function CalendarView({ proposal, onProposalChanged, onRegenerate, readonly }: Props) {
  const [teachers, setTeachers] = useState<Teacher[]>([]);
  const [qualifiedPairs, setQualifiedPairs] = useState<Set<string>>(new Set());
  const [selectedDay, setSelectedDay] = useState<string | null>(null);
  const [blocks, setBlocks] = useState<AvailabilityBlock[]>([]);
  const [externalShifts, setExternalShifts] = useState<ExternalShiftRow[]>([]);

  useEffect(() => {
    const month = proposal.summary.target_month;
    api.listTeachers().then(setTeachers);
    api.listQualifiedPairs().then((list) => setQualifiedPairs(new Set(list)));
    api.listAvailabilityBlocks(month).then(setBlocks);
    api.listExternalShiftsForMonth(month).then(setExternalShifts);
  }, [proposal.summary.target_month, proposal.summary.id]);

  const issues: Issue[] = useMemo(
    () => computeIssues(
      proposal.shifts, teachers, qualifiedPairs, blocks, externalShifts, []
    ),
    [proposal.shifts, teachers, qualifiedPairs, blocks, externalShifts],
  );

  const issueShiftIds = useMemo(
    () => new Set(issues.map((w) => w.shift_id).filter((id): id is number => id != null)),
    [issues],
  );

  const todayIso = new Date().toISOString().slice(0, 10);
  const targetMonth = proposal.summary.target_month.slice(0, 7);

  const dayShifts = selectedDay
    ? proposal.shifts.filter((s) => s.shift_date === selectedDay)
    : [];
  const dayWarnings = selectedDay
    ? issues.filter((w) => w.shift_date === selectedDay)
    : [];

  const handleSave = async (proposalShiftId: number, newUserId: number | null) => {
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
    <div style={{ display: "flex", gap: "0.5rem" }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        {proposal.is_stale && proposal.last_pulled_at && (
          <StaleBanner
            lastPulledAt={proposal.last_pulled_at}
            generatedAt={proposal.summary.generated_at}
            onRegenerate={onRegenerate}
          />
        )}
        <CalendarHeader targetMonth={targetMonth} />
        <MonthGrid
          targetMonth={targetMonth}
          shifts={proposal.shifts}
          warningShiftIds={issueShiftIds}
          selectedDay={selectedDay}
          todayIso={todayIso}
          onDayClick={handleDayClick}
          onSlotClick={handleSlotClick}
        />
        {selectedDay && (
          <DayPanel
            iso={selectedDay}
            shifts={dayShifts}
            teachers={teachers}
            qualifiedPairs={qualifiedPairs}
            warnings={dayWarnings}
            onClose={() => setSelectedDay(null)}
            onSave={handleSave}
          />
        )}
      </div>
      <IssueQueue
        issues={issues}
        shifts={proposal.shifts}
        teachers={teachers}
        qualifiedPairs={qualifiedPairs}
        blocks={blocks}
        readonly={!!readonly}
        onApplySwap={async (shiftId, userId) => {
          await api.editProposalShiftTeacher(shiftId, userId, null);
          onProposalChanged();
        }}
        onImportExternal={async (slingShiftId) => {
          await api.importExternalShift(slingShiftId, proposal.summary.id);
          onProposalChanged();
          // Also refresh external_shifts so the imported one isn't flagged again
          api.listExternalShiftsForMonth(proposal.summary.target_month).then(setExternalShifts);
        }}
        onOpenDay={(iso) => setSelectedDay(iso)}
      />
    </div>
  );
}
