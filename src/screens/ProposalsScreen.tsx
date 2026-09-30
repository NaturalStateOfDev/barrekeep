import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Clock,
  Download,
  GitBranchPlus,
  PlugZap,
  RefreshCw,
  Scale,
  Sparkles,
  Upload,
} from "lucide-react";
import { api } from "../lib/api";
import { CalendarView } from "../components/calendar/CalendarView";
import { ClaudeEditorPanel } from "../components/claude/ClaudeEditorPanel";
import { AlgorithmCard } from "../components/claude/AlgorithmCard";
import { VersionProposalCard } from "../components/claude/VersionProposalCard";
import { SlingTokenModal } from "../components/SlingTokenModal";
import { PushModal } from "../components/PushModal";
import { DraftNameModal } from "../components/DraftNameModal";
import { CompareView } from "../components/CompareView";
import { MonthSelector } from "../components/MonthSelector";
import { ProposalSwitcher, type MonthEntry } from "../components/ui/ProposalSwitcher";
import { DraftSwitcher } from "../components/ui/DraftSwitcher";
import { PageHead } from "../components/ui/PageHead";
import { Kpi, CoverageRing } from "../components/ui/Kpi";
import { Tabs } from "../components/ui/Tabs";
import { EmptyState } from "../components/ui/EmptyState";
import { LoadingBlock } from "../components/ui/LoadingBlock";
import { Avatar } from "../components/ui/Avatar";
import { ClassChip } from "../components/ui/ClassChip";
import { computeIssues, type Issue } from "../lib/issues";
import { computeKpis } from "../lib/kpis";
import { codifyInstruction } from "../lib/rules";
import { draftsForMonth, pushDraftFor, representativeDraft } from "../lib/drafts";
import { pushLabel } from "../lib/sync";
import { isStudioNotConfigured, openStudioSetup } from "../lib/studioSetup";
import { useTimeFormat } from "../lib/timeFormat";
import {
  monthWindow,
  isReadOnlyMonth,
  monthLabel,
  WEEKDAYS_SHORT,
} from "../lib/dates";
import type {
  ClaudeEditResult,
  Position,
  Teacher,
  ProposalSummary,
  ProposalDetail,
  EditRow,
  ReviewSuggestion,
  ReviewRunSummary,
  AvailabilityBlock,
  ExternalShiftRow,
  DraftConflict,
} from "../types";

function todayIso(): string {
  return new Date().toISOString().slice(0, 10);
}

const TABS = ["calendar", "list", "edits", "compare", "claude"] as const;
type Tab = (typeof TABS)[number];

export function ProposalsScreen({ onGoSettings }: { onGoSettings: () => void }) {
  const today = todayIso();
  const tf = useTimeFormat();
  const [proposals, setProposals] = useState<ProposalSummary[] | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [mode, setMode] = useState<"detail" | "new">("detail");
  const [detail, setDetail] = useState<ProposalDetail | null>(null);
  const [hasToken, setHasToken] = useState<boolean | null>(null);
  const [tab, setTab] = useState<Tab>("calendar");
  const [generating, setGenerating] = useState(false);
  const [pulling, setPulling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastResult, setLastResult] = useState<string | null>(null);
  const [pullResult, setPullResult] = useState<string | null>(null);
  const [slingExpiredModal, setSlingExpiredModal] = useState(false);
  const [pushOpen, setPushOpen] = useState(false);
  // "Remove this draft's shifts from Sling" (a pushed, non-push draft).
  const [removeOpen, setRemoveOpen] = useState(false);
  // Availability refresh in place: the re-check result for one draft.
  const [refreshingAvail, setRefreshingAvail] = useState(false);
  const [conflicts, setConflicts] = useState<{ proposalId: number; list: DraftConflict[] } | null>(null);
  // Viewing a draft that isn't the month's push draft and clicked Push.
  const [pushGateOpen, setPushGateOpen] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const [nameModal, setNameModal] = useState<null | "generate" | "duplicate" | "rename">(null);

  const [newMonth, setNewMonth] = useState<string>(() => {
    const [y, m] = today.split("-").map(Number);
    return m === 12 ? `${y + 1}-01` : `${y}-${String(m + 1).padStart(2, "0")}`;
  });

  // Schedule context for the selected proposal (shared by KPIs, the
  // calendar grid, the issue queue and the day editor).
  const [teachers, setTeachers] = useState<Teacher[]>([]);
  const [positions, setPositions] = useState<Position[]>([]);
  const [hasAnthropicKey, setHasAnthropicKey] = useState(false);
  const [algoRefresh, setAlgoRefresh] = useState(0);
  const [qualifiedPairs, setQualifiedPairs] = useState<Set<string>>(new Set());
  const [blocks, setBlocks] = useState<AvailabilityBlock[]>([]);
  const [externalShifts, setExternalShifts] = useState<ExternalShiftRow[]>([]);

  const refreshProposals = async () => {
    const list = await api.listProposals();
    setProposals(list);
    return list;
  };

  const refreshDetail = async (id: number) => {
    setDetail(await api.getProposal(id));
  };

  useEffect(() => {
    api.hasSlingToken().then(setHasToken).catch(() => setHasToken(null));
    refreshProposals()
      .then((list) => {
        // Newest month's push draft (the list is newest-created first).
        const next = list[0] ? representativeDraft(list, list[0].target_month)?.id ?? null : null;
        setSelectedId(next);
        if (next == null) setMode("new");
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (selectedId == null) return;
    refreshDetail(selectedId).catch((e) => setError(String(e)));
  }, [selectedId]);

  const shownConflicts = conflicts && conflicts.proposalId === selectedId ? conflicts.list : null;

  useEffect(() => {
    setPullResult(null);
  }, [newMonth]);

  const loadContext = (month: string) => {
    api.listTeachers().then(setTeachers).catch(() => {});
    api.listPositions().then(setPositions).catch(() => {});
    api.hasAnthropicKey().then(setHasAnthropicKey).catch(() => {});
    api.listQualifiedPairs().then((list) => setQualifiedPairs(new Set(list))).catch(() => {});
    api.listAvailabilityBlocks(month).then(setBlocks).catch(() => {});
    api.listExternalShiftsForMonth(month).then(setExternalShifts).catch(() => {});
  };

  useEffect(() => {
    if (!detail) return;
    loadContext(detail.summary.target_month);
  }, [detail?.summary.target_month, detail?.summary.id]);

  // Default new-month per spec: first empty in priority next > next+1 >
  // current > previous. Fires once, on the first proposals load.
  const defaultMonthPicked = useRef(false);
  useEffect(() => {
    if (!proposals || defaultMonthPicked.current) return;
    defaultMonthPicked.current = true;
    const window = monthWindow(today);
    const priority = [window[2], window[3], window[1], window[0]];
    const proposedMonths = new Set(proposals.map((p) => p.target_month));
    const firstEmpty = priority.find((m) => !proposedMonths.has(m));
    if (firstEmpty) setNewMonth(firstEmpty);
  }, [proposals]);

  const issues: Issue[] = useMemo(
    () =>
      detail
        ? computeIssues(detail.shifts, teachers, qualifiedPairs, blocks, externalShifts, [], tf.fmt)
        : [],
    [detail, teachers, qualifiedPairs, blocks, externalShifts, tf.fmt],
  );

  const kpis = useMemo(() => computeKpis(detail?.shifts ?? []), [detail]);

  // One switcher entry per month, newest month first. Picking a month opens
  // its push draft (the draft Push sends), else its newest draft.
  const monthEntries = useMemo<MonthEntry[]>(() => {
    const months = [...new Set((proposals ?? []).map((p) => p.target_month))].sort((a, b) =>
      b.localeCompare(a),
    );
    return months.map((month) => {
      const all = (proposals ?? []).filter((p) => p.target_month === month);
      return {
        month,
        proposalId: representativeDraft(all, month)!.id,
        draftCount: all.filter((p) => !p.archived).length,
      };
    });
  }, [proposals]);

  const selectedSummary =
    selectedId != null ? proposals?.find((p) => p.id === selectedId) : undefined;
  const selectedMonth = selectedSummary?.target_month ?? null;
  // Drafts listed in the switcher (archived only when shown, or when viewed).
  const monthDrafts = useMemo(
    () =>
      selectedMonth ? draftsForMonth(proposals ?? [], selectedMonth, showArchived, selectedId) : [],
    [proposals, selectedMonth, showArchived, selectedId],
  );
  // Drafts a Claude prompt / Compare can target: never archived ones (but
  // always the one on screen).
  const activeMonthDrafts = useMemo(
    () => (selectedMonth ? draftsForMonth(proposals ?? [], selectedMonth, false, selectedId) : []),
    [proposals, selectedMonth, selectedId],
  );
  const archivedCount = selectedMonth
    ? (proposals ?? []).filter((p) => p.target_month === selectedMonth && p.archived).length
    : 0;
  const pushDraft = selectedMonth ? pushDraftFor(proposals ?? [], selectedMonth) : undefined;

  const activeMonth = mode === "detail" && detail ? detail.summary.target_month : newMonth;
  const readonly = isReadOnlyMonth(activeMonth, today);
  const readonlyTitle = readonly ? "Past month — read only" : "";

  const onPull = async () => {
    setError(null);
    setPullResult(null);
    setPulling(true);
    try {
      const r = await api.pullMonthFromSling(activeMonth);
      setPullResult(
        `Pulled ${activeMonth}: ${r.user_count} users, ${r.qual_count} qualifications, ` +
          `${r.availability_count} availability blocks, ${r.external_shift_count} external shifts, ` +
          `${r.history_shift_count} trailing-history shifts.`,
      );
      await refreshProposals();
      if (mode === "detail" && selectedId != null) await refreshDetail(selectedId);
      // A pull rewrites roster, availability and external shifts — reload the
      // issue/KPI context so the queue reflects the fresh data.
      loadContext(activeMonth);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("sling-401")) setSlingExpiredModal(true);
      else setError(msg);
    } finally {
      setPulling(false);
    }
  };

  // Re-pull availability/leave (+ roster) for the current and future months,
  // then re-check THIS draft against it — no regeneration, edits kept.
  const onRefreshAvailability = async () => {
    if (selectedId == null) return;
    setError(null);
    setPullResult(null);
    setRefreshingAvail(true);
    try {
      const r = await api.refreshAvailabilityFromSling();
      const list = await api.checkDraftConflicts(selectedId);
      setConflicts({ proposalId: selectedId, list });
      setPullResult(
        `Refreshed availability for ${r.months.map((m) => monthLabel(m.target_month)).join(", ")} ` +
          `(${r.months.reduce((n, m) => n + m.availability_count, 0)} availability/leave blocks). ` +
          (list.length === 0
            ? "This draft has no conflicts."
            : `${list.length} conflict${list.length === 1 ? "" : "s"} in this draft — see the list above the calendar.`),
      );
      await refreshProposals();
      await refreshDetail(selectedId);
      if (detail) loadContext(detail.summary.target_month);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("sling-401")) setSlingExpiredModal(true);
      else setError(msg);
    } finally {
      setRefreshingAvail(false);
    }
  };

  const onGenerate = async (name?: string) => {
    if (isReadOnlyMonth(activeMonth, today)) return;
    setError(null);
    setLastResult(null);
    setGenerating(true);
    try {
      const result = await api.generateProposal(activeMonth, name || undefined);
      const list = await refreshProposals();
      const made = list.find((p) => p.id === result.proposal_id);
      const push = pushDraftFor(list, result.target_month);
      setLastResult(
        `Generated “${made?.name ?? `Draft #${result.proposal_id}`}” for ${result.target_month} ` +
          `(${result.algorithm_version}, ${result.shift_count} shifts, ` +
          `${result.dropped_count} dropped)` +
          (push && push.id !== result.proposal_id
            ? `. The push draft is still “${push.name}” — use “Use for push” in the draft menu to change it.`
            : ""),
      );
      setSelectedId(result.proposal_id);
      setMode("detail");
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
    }
  };

  // ---- Draft actions (duplicate / rename / archive / use for push) ----
  const runDraftAction = async (fn: () => Promise<void>) => {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(String(e));
    }
  };

  const onDuplicate = async (name: string) => {
    if (!selectedSummary) return;
    const id = await api.duplicateProposal(selectedSummary.id, name);
    await refreshProposals();
    setSelectedId(id);
    setNameModal(null);
    setLastResult(`Duplicated “${selectedSummary.name}” as “${name}”. Changes to the copy leave the original alone.`);
  };

  const onRename = async (name: string) => {
    if (!selectedSummary) return;
    await api.renameProposal(selectedSummary.id, name);
    await onProposalChanged();
    setNameModal(null);
  };

  const onArchiveToggle = () =>
    runDraftAction(async () => {
      if (!selectedSummary) return;
      if (selectedSummary.archived) {
        await api.unarchiveProposal(selectedSummary.id);
        await onProposalChanged();
      } else {
        await api.archiveProposal(selectedSummary.id);
        const list = await refreshProposals();
        // Move off the archived draft unless archived drafts are shown.
        if (!showArchived) {
          const next = representativeDraft(
            list.filter((p) => !p.archived),
            selectedSummary.target_month,
          );
          if (next) setSelectedId(next.id);
        }
      }
    });

  const onUseForPush = (id: number) =>
    runDraftAction(async () => {
      const d = proposals?.find((p) => p.id === id);
      if (!d) return;
      await api.setPushCandidate(d.target_month, id);
      await onProposalChanged();
      setLastResult(`“${d.name}” is now the push draft for ${monthLabel(d.target_month)}.`);
    });

  const onPushClick = () => {
    if (detail?.summary.is_push_candidate) setPushOpen(true);
    else setPushGateOpen(true);
  };

  const onProposalChanged = async () => {
    try {
      if (selectedId != null) await refreshDetail(selectedId);
      await refreshProposals();
      // Keep a shown conflict list current as the user fixes slots (DB only).
      if (selectedId != null && conflicts?.proposalId === selectedId) {
        setConflicts({ proposalId: selectedId, list: await api.checkDraftConflicts(selectedId) });
      }
    } catch (e) {
      setError(String(e));
    }
  };

  // ---- First run, not connected: teach the next step ----
  if (proposals && proposals.length === 0 && hasToken === false) {
    return (
      <div>
        <PageHead title="Proposals" />
        <div className="card">
          <EmptyState
            icon={PlugZap}
            title="Not connected to Sling"
            message="Barrekeep builds proposals from your Sling roster, qualifications and availability. Connect Sling in Settings to pull your studio's data."
            actionLabel="Open Settings"
            onAction={onGoSettings}
          />
        </div>
      </div>
    );
  }

  // Initial list load failed (e.g. database locked): show the error instead
  // of a blank page.
  if (!proposals) {
    if (!error) return null;
    return (
      <div>
        <PageHead title="Proposals" />
        <div className="card error" style={{ marginTop: 0 }}>{error}</div>
      </div>
    );
  }

  // The draft id + current/superseded status now live in the version pill
  // next to the title, so the subline keeps only the schedule stats.
  const summary = detail?.summary;
  const subline =
    mode === "detail" && summary ? (
      <>
        {kpis.totalCount} classes · {kpis.teacherCount} teachers
        {summary.edit_count > 0 &&
          ` · ${summary.edit_count} manual edit${summary.edit_count === 1 ? "" : "s"}`}
        {readonly && " · past month, read only"}
      </>
    ) : undefined;

  return (
    <div>
      <PageHead
        title={
          <div className="bk-title-row">
            <ProposalSwitcher
              months={monthEntries}
              value={mode === "detail" ? selectedSummary?.target_month ?? null : null}
              fallbackTitle={monthLabel(newMonth)}
              onChange={(id) => {
                setSelectedId(id);
                setMode("detail");
              }}
              onNew={() => {
                setMode("new");
                setError(null);
                setLastResult(null);
              }}
            />
            {mode === "detail" && selectedId != null && monthDrafts.length > 0 && (
              <DraftSwitcher
                drafts={monthDrafts}
                archivedCount={archivedCount}
                showArchived={showArchived}
                onToggleArchived={() => setShowArchived((v) => !v)}
                value={selectedId}
                onChange={(id) => setSelectedId(id)}
                readonly={readonly}
                onDuplicate={() => setNameModal("duplicate")}
                onRename={() => setNameModal("rename")}
                onArchiveToggle={onArchiveToggle}
                onUseForPush={() => onUseForPush(selectedId)}
                onRemoveFromSling={() => setRemoveOpen(true)}
                onCompare={() => setTab("compare")}
              />
            )}
          </div>
        }
        sub={subline}
        actions={
          mode === "detail" && detail ? (
            <>
              <button className="btn-ghost" onClick={onPull} disabled={pulling || readonly} title={readonlyTitle}>
                <Download size={15} /> {pulling ? "Pulling…" : "Pull"}
              </button>
              <button
                className="btn-ghost"
                onClick={() => setNameModal("generate")}
                disabled={generating || readonly}
                title={readonly ? readonlyTitle : "Generate another draft for this month"}
              >
                <Sparkles size={15} /> {generating ? "Generating…" : "New draft"}
              </button>
              <button
                className="btn-primary"
                onClick={onPushClick}
                disabled={readonly}
                title={
                  readonly
                    ? readonlyTitle
                    : detail.summary.is_push_candidate
                      ? detail.summary.sling_shift_count > 0
                        ? `Send the changes to “${detail.summary.name}” since the last push (planning shifts only)`
                        : `Push “${detail.summary.name}” to Sling as planning shifts`
                      : `Push sends the push draft${pushDraft ? ` (“${pushDraft.name}”)` : ""}, not this one`
                }
              >
                <Upload size={15} /> {detail.summary.is_push_candidate ? pushLabel(detail.summary) : "Push…"}
              </button>
            </>
          ) : undefined
        }
      />

      {pullResult && <div className="ok" style={{ margin: "0 0 14px" }}>{pullResult}</div>}
      {lastResult && <div className="ok" style={{ margin: "0 0 14px" }}>{lastResult}</div>}
      {error && (
        <div className="error" style={{ margin: "0 0 14px" }}>
          {error}
          {isStudioNotConfigured(error) && (
            <>
              {" "}
              <button className="btn-link" onClick={openStudioSetup}>Set up studio</button>
            </>
          )}
        </div>
      )}

      {mode === "new" ? (
        <div className="card">
          {generating ? (
            <LoadingBlock label={`Generating proposal for ${monthLabel(newMonth)}…`} />
          ) : (
            <>
              <div className="row" style={{ justifyContent: "center" }}>
                <label className="field" style={{ marginBottom: 0 }}>
                  <span>Target month</span>
                  <MonthSelector today={today} value={newMonth} onChange={setNewMonth} />
                </label>
                <button
                  className="btn-ghost"
                  style={{ alignSelf: "flex-end" }}
                  onClick={onPull}
                  disabled={pulling || isReadOnlyMonth(newMonth, today)}
                  title={isReadOnlyMonth(newMonth, today) ? "Past month — read only" : ""}
                >
                  <Download size={15} /> {pulling ? "Pulling…" : `Pull from Sling`}
                </button>
              </div>
              {isReadOnlyMonth(newMonth, today) ? (
                <EmptyState
                  icon={Sparkles}
                  title={`No proposal for ${monthLabel(newMonth)}`}
                  message="Past month — read only. Pick the current or an upcoming month to generate a proposal."
                />
              ) : (
                <EmptyState
                  icon={Sparkles}
                  title={`No proposal for ${monthLabel(newMonth)} yet`}
                  message="Pull the latest availability, then generate a first draft from your Sling roster and qualifications. Review and adjust it here before pushing."
                  actionLabel={`Generate proposal for ${newMonth}`}
                  onAction={() => onGenerate()}
                />
              )}
            </>
          )}
        </div>
      ) : generating ? (
        <div className="card">
          <LoadingBlock label="Regenerating proposal…" />
        </div>
      ) : detail ? (
        <>
          <div className="bk-kpi-grid">
            <Kpi label="Coverage" value={kpis.coveragePct} unit="%" ring={<CoverageRing pct={kpis.coveragePct} />} />
            <Kpi
              label="Load balance"
              value={kpis.balance}
              icon={Scale}
              tint="var(--color-warning-bg)"
              ink={kpis.balance === "Uneven" ? "var(--color-warning)" : "var(--text-body)"}
            />
            <Kpi
              label="Open conflicts"
              value={issues.length}
              icon={AlertTriangle}
              tint={issues.length > 0 ? "var(--color-danger-bg)" : "var(--color-success-bg)"}
              ink={issues.length > 0 ? "var(--color-danger)" : "var(--color-success)"}
            />
            <Kpi label="Teacher hours" value={kpis.teacherHours} unit="h" icon={Clock} tint="var(--accent-soft)" ink="var(--accent)" />
          </div>

          <Tabs tabs={TABS} value={tab} onChange={setTab} />

          {tab === "calendar" && (
            <CalendarView
              proposal={detail}
              teachers={teachers}
              positions={positions}
              qualifiedPairs={qualifiedPairs}
              blocks={blocks}
              issues={issues}
              onProposalChanged={onProposalChanged}
              onRegenerate={() => onGenerate()}
              onRefreshAvailability={onRefreshAvailability}
              refreshing={refreshingAvail}
              conflicts={shownConflicts}
              onDismissConflicts={() => setConflicts(null)}
              onImportExternal={async (slingShiftId) => {
                await api.importExternalShift(slingShiftId, detail.summary.id);
                await onProposalChanged();
                api.listExternalShiftsForMonth(detail.summary.target_month).then(setExternalShifts).catch(() => {});
              }}
              readonly={readonly}
            />
          )}
          {tab === "list" && <ProposalShiftsTable detail={detail} />}
          {tab === "edits" && <EditHistory proposalId={detail.summary.id} />}
          {tab === "compare" && (
            <CompareView
              drafts={activeMonthDrafts}
              viewingId={detail.summary.id}
              onOpenDraft={(id) => {
                setSelectedId(id);
                setTab("calendar");
              }}
            />
          )}
          {tab === "claude" && (
            <>
              <ClaudeEditorPanel
                detail={detail}
                positions={positions}
                teachers={teachers}
                hasKey={hasAnthropicKey}
                readonly={readonly}
                monthDrafts={activeMonthDrafts}
                onProposalChanged={onProposalChanged}
                onDraftsChanged={() => {
                  refreshProposals().catch((e) => setError(String(e)));
                }}
                onOpenDraft={(id) => {
                  setSelectedId(id);
                  setTab("calendar");
                }}
                onVersionAdopted={() => setAlgoRefresh((n) => n + 1)}
              />
              <ClaudeReviewSection
                proposalId={detail.summary.id}
                teachers={teachers}
                onVersionAdopted={() => setAlgoRefresh((n) => n + 1)}
              />
              <AlgorithmCard refreshToken={algoRefresh} teachers={teachers} />
            </>
          )}
        </>
      ) : null}

      {slingExpiredModal && (
        <SlingTokenModal
          reason="expired"
          onSaved={() => setSlingExpiredModal(false)}
          onCancel={() => setSlingExpiredModal(false)}
        />
      )}
      {nameModal === "generate" && (
        <DraftNameModal
          title={`New draft for ${monthLabel(activeMonth)}`}
          initial=""
          optional
          placeholder={`Draft ${(proposals ?? []).filter((p) => p.target_month === activeMonth).length + 1}`}
          confirmLabel="Generate"
          hint={
            pushDraft ? (
              <>
                Runs the active algorithm again as a separate draft. The push draft stays “
                {pushDraft.name}” until you choose “Use for push” on another draft.
              </>
            ) : undefined
          }
          onConfirm={async (name) => {
            setNameModal(null);
            await onGenerate(name);
          }}
          onCancel={() => setNameModal(null)}
        />
      )}
      {nameModal === "duplicate" && selectedSummary && (
        <DraftNameModal
          title={`Duplicate “${selectedSummary.name}”`}
          initial={`Copy of ${selectedSummary.name}`.slice(0, 60)}
          confirmLabel="Duplicate"
          hint="Copies every class and assignment (not the edit history). Try a what-if on the copy, then compare."
          onConfirm={onDuplicate}
          onCancel={() => setNameModal(null)}
        />
      )}
      {nameModal === "rename" && selectedSummary && (
        <DraftNameModal
          title="Rename draft"
          initial={selectedSummary.name}
          confirmLabel="Rename"
          onConfirm={onRename}
          onCancel={() => setNameModal(null)}
        />
      )}
      {pushGateOpen && detail && (
        <div className="modal-backdrop" onClick={() => setPushGateOpen(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>Push sends the push draft</h3>
            <p className="muted" style={{ marginTop: 0 }}>
              You're viewing “{detail.summary.name}”.{" "}
              {pushDraft ? (
                <>
                  The push draft for {monthLabel(detail.summary.target_month)} is “{pushDraft.name}”.
                </>
              ) : (
                <>{monthLabel(detail.summary.target_month)} has no push draft yet.</>
              )}{" "}
              Only one draft per month goes to Sling. After switching, the push keeps shifts the two
              drafts share and offers to remove the earlier draft's other planning shifts.
            </p>
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 18, flexWrap: "wrap" }}>
              <button className="btn-ghost" onClick={() => setPushGateOpen(false)}>
                Cancel
              </button>
              {pushDraft && (
                <button
                  className="btn-ghost"
                  onClick={() => {
                    setPushGateOpen(false);
                    setSelectedId(pushDraft.id);
                  }}
                >
                  Switch to “{pushDraft.name}”
                </button>
              )}
              <button
                className="btn-primary"
                onClick={async () => {
                  setPushGateOpen(false);
                  await onUseForPush(detail.summary.id);
                  setPushOpen(true);
                }}
              >
                Use “{detail.summary.name}” for push
              </button>
            </div>
          </div>
        </div>
      )}
      {removeOpen && detail && (
        <PushModal
          mode="remove"
          proposalId={detail.summary.id}
          draftName={detail.summary.name}
          monthLabel={monthLabel(detail.summary.target_month)}
          onClose={() => {
            setRemoveOpen(false);
            onProposalChanged();
          }}
          onTokenExpired={() => {
            setRemoveOpen(false);
            setSlingExpiredModal(true);
          }}
        />
      )}
      {pushOpen && detail && (
        <PushModal
          mode="push"
          proposalId={detail.summary.id}
          draftName={detail.summary.name}
          monthLabel={monthLabel(detail.summary.target_month)}
          onClose={() => {
            setPushOpen(false);
            onProposalChanged();
          }}
          onTokenExpired={() => {
            setPushOpen(false);
            setSlingExpiredModal(true);
          }}
        />
      )}
    </div>
  );
}

function ProposalShiftsTable({ detail }: { detail: ProposalDetail }) {
  const { summary, shifts } = detail;
  const tf = useTimeFormat();
  return (
    <div className="card">
      <div className="row">
        <strong>
          Proposal #{summary.id} — {summary.target_month} ({summary.algorithm_version})
        </strong>
        {summary.edit_count > 0 && (
          <span className="badge">
            {summary.edit_count} manual edit{summary.edit_count === 1 ? "" : "s"}
          </span>
        )}
        <span className="muted" style={{ marginLeft: "auto", fontSize: 12 }}>
          Read-only view. Edit teachers from the calendar tab.
        </span>
      </div>
      <table>
        <thead>
          <tr>
            <th>Date</th>
            <th>Day</th>
            <th>Time</th>
            <th>Class</th>
            <th>Teacher</th>
            <th>Reason</th>
            <th>Flag</th>
          </tr>
        </thead>
        <tbody>
          {shifts.map((s) => (
            <tr key={s.id} className={s.is_dropped ? "dropped" : ""}>
              <td>{s.shift_date}</td>
              <td className="muted">{weekday(s.shift_date)}</td>
              <td>
                {tf.range(s.start_time, s.end_time)}
              </td>
              <td>
                <ClassChip className={s.class_name} size="md" />
              </td>
              <td>
                {s.is_coteach ? (
                  <strong>{s.coteach_label}</strong>
                ) : s.is_dropped ? (
                  <span className="muted">Dropped</span>
                ) : s.teacher_name ? (
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 7 }}>
                    <Avatar name={s.teacher_name} size={20} />
                    {s.teacher_name}
                  </span>
                ) : (
                  <span style={{ color: "var(--color-danger)", fontWeight: 600 }}>Unassigned</span>
                )}
              </td>
              <td className="muted">{s.generation_reason}</td>
              <td className="muted">{s.flag ?? ""}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function EditHistory({ proposalId }: { proposalId: number }) {
  const [edits, setEdits] = useState<EditRow[] | null>(null);
  const tf = useTimeFormat();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .listEditsForProposal(proposalId)
      .then(setEdits)
      .catch((e) => setError(String(e)));
  }, [proposalId]);

  if (error) return <div className="card error">{error}</div>;
  if (!edits || edits.length === 0) {
    return (
      <div className="card">
        <span className="muted">No manual edits on this proposal yet.</span>
      </div>
    );
  }

  return (
    <div className="card">
      <strong>
        Edit history ({edits.length})
      </strong>
      <table style={{ marginTop: 10 }}>
        <thead>
          <tr>
            <th>When</th>
            <th>Slot</th>
            <th>Class</th>
            <th>From</th>
            <th>To</th>
            <th>Reason</th>
          </tr>
        </thead>
        <tbody>
          {edits.map((e) => (
            <tr key={e.id} className={e.reverted ? "dropped" : ""}>
              <td className="muted">{tf.timestamp(e.edited_at)}</td>
              <td>
                {e.shift_date} {tf.time(e.start_time)}
              </td>
              <td>{e.class_name}</td>
              <td>
                {e.field === "sling_position_id"
                  ? e.old_class_name ?? e.old_value
                  : e.old_teacher_name ?? <span className="muted">Dropped</span>}
              </td>
              <td>
                {e.field === "sling_position_id" ? (
                  <>
                    {e.new_class_name ?? e.new_value}{" "}
                    <span className="pill pill-fyi">format</span>
                  </>
                ) : (
                  e.new_teacher_name ?? <span className="muted">Dropped</span>
                )}
              </td>
              <td className="muted">{e.reason ?? ""}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ClaudeReviewSection({
  proposalId,
  teachers,
  onVersionAdopted,
}: {
  proposalId: number;
  teachers: Teacher[];
  onVersionAdopted: () => void;
}) {
  const [reviews, setReviews] = useState<ReviewRunSummary[] | null>(null);
  const tf = useTimeFormat();
  const [hasKey, setHasKey] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const [list, keyOk] = await Promise.all([
        api.listReviewsForProposal(proposalId),
        api.hasAnthropicKey(),
      ]);
      setReviews(list);
      setHasKey(keyOk);
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [proposalId]);

  const onReview = async () => {
    setError(null);
    setRunning(true);
    try {
      await api.reviewProposal(proposalId);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  const latest = reviews && reviews.length > 0 ? reviews[0] : null;

  return (
    <div className="card">
      <div className="row">
        <strong>Claude review</strong>
        <button
          className="btn-primary"
          onClick={onReview}
          disabled={running || !hasKey}
          style={{ marginLeft: "auto" }}
          title={hasKey ? "" : "Set your API key in Settings first"}
        >
          <RefreshCw size={15} /> {running ? "Reviewing…" : latest ? "Run again" : "Have Claude review"}
        </button>
      </div>
      {!hasKey && (
        <div className="muted" style={{ marginTop: 8 }}>
          Set your Anthropic API key in Settings to enable this.
        </div>
      )}
      {error && <div className="error">{error}</div>}

      {latest && (
        <div style={{ marginTop: 16 }}>
          <div className="muted" style={{ fontSize: 12 }}>
            {latest.model} · {latest.input_tokens.toLocaleString()} in /{" "}
            {latest.output_tokens.toLocaleString()} out · ${latest.cost_usd.toFixed(4)} ·{" "}
            {(latest.duration_ms / 1000).toFixed(1)}s · {tf.timestamp(latest.ran_at)}
          </div>
          <p style={{ marginTop: 12 }}>{latest.overall_assessment}</p>
          {latest.suggestions.length === 0 ? (
            <div className="muted">No suggestions — Claude thinks the schedule is fine as-is.</div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              {latest.suggestions.map((s, i) => (
                <SuggestionCard
                  key={`${latest.id}-${i}`}
                  s={s}
                  proposalId={proposalId}
                  hasKey={hasKey}
                  teachers={teachers}
                  onVersionAdopted={onVersionAdopted}
                />
              ))}
            </div>
          )}
        </div>
      )}

      {reviews && reviews.length > 1 && (
        <details style={{ marginTop: 16 }}>
          <summary className="muted" style={{ cursor: "pointer" }}>
            {reviews.length - 1} earlier review{reviews.length - 1 === 1 ? "" : "s"}
          </summary>
          <div style={{ display: "flex", flexDirection: "column", gap: 16, marginTop: 8 }}>
            {reviews.slice(1).map((r) => (
              <div
                key={r.id}
                className="muted"
                style={{ fontSize: 12, paddingLeft: 12, borderLeft: "2px solid var(--border-hairline)" }}
              >
                <div>
                  {tf.timestamp(r.ran_at)} · {r.model} · ${r.cost_usd.toFixed(4)}
                </div>
                <div style={{ marginTop: 4 }}>{r.overall_assessment}</div>
                <div style={{ marginTop: 4 }}>
                  {r.suggestions.length} suggestion{r.suggestions.length === 1 ? "" : "s"}
                </div>
              </div>
            ))}
          </div>
        </details>
      )}
    </div>
  );
}

function SuggestionCard({
  s,
  proposalId,
  hasKey,
  teachers,
  onVersionAdopted,
}: {
  s: ReviewSuggestion;
  proposalId: number;
  hasKey: boolean;
  teachers: Teacher[];
  onVersionAdopted: () => void;
}) {
  const kindLabel: Record<string, string> = {
    add_rule: "Add rule",
    tweak_parameter: "Tweak parameter",
    fyi: "FYI",
  };
  const actionable = s.type === "add_rule" || s.type === "tweak_parameter";
  const [codifying, setCodifying] = useState(false);
  const [result, setResult] = useState<ClaudeEditResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);

  const onCodify = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setCodifying(true);
    setError(null);
    setResult(null);
    try {
      setResult(await api.claudeEditProposal(proposalId, codifyInstruction(s)));
    } catch (e) {
      setError(String(e));
    } finally {
      inFlight.current = false;
      setCodifying(false);
    }
  };

  return (
    <div className="suggestion">
      <div className="row" style={{ marginBottom: 4 }}>
        <span className={`pill pill-${s.type}`}>{kindLabel[s.type] ?? s.type}</span>
        <span className={`pill pill-confidence pill-${s.confidence}`}>{s.confidence}</span>
        {actionable && (
          <button
            className="btn-ghost btn-sm"
            style={{ marginLeft: "auto" }}
            onClick={onCodify}
            disabled={codifying || !hasKey}
            title={hasKey ? "Ask Claude to express this as a rule, then review and adopt it" : "Set your API key in Settings first"}
          >
            <GitBranchPlus size={14} /> {codifying ? "Drafting rule…" : result ? "Draft again" : "Make it a rule"}
          </button>
        )}
      </div>
      <div style={{ fontWeight: 600 }}>{s.summary}</div>
      <div className="muted" style={{ fontSize: 13, marginTop: 4 }}>
        {s.rationale}
      </div>
      {codifying && <LoadingBlock label="Asking Claude for a rule proposal…" />}
      {error && <div className="error">{error}</div>}
      {result && !codifying && (
        result.ruleset_proposal ? (
          <VersionProposalCard
            proposal={result.ruleset_proposal}
            runId={result.run_id}
            teachers={teachers}
            onAdopted={onVersionAdopted}
          />
        ) : (
          <div className="bk-warn">
            {result.needs_code_change
              ? `The rule keys can't express this: ${result.needs_code_change.rationale} Ask Claude in the box above to draft a code change instead.`
              : result.summary}
          </div>
        )
      )}
    </div>
  );
}

function weekday(isoDate: string): string {
  const d = new Date(isoDate + "T00:00:00");
  return WEEKDAYS_SHORT[d.getDay()];
}
