import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Clock,
  Download,
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
import { SlingTokenModal } from "../components/SlingTokenModal";
import { PushModal } from "../components/PushModal";
import { MonthSelector } from "../components/MonthSelector";
import { ProposalSwitcher, type MonthEntry } from "../components/ui/ProposalSwitcher";
import { VersionSwitcher } from "../components/ui/VersionSwitcher";
import { PageHead } from "../components/ui/PageHead";
import { Kpi, CoverageRing } from "../components/ui/Kpi";
import { Tabs } from "../components/ui/Tabs";
import { EmptyState } from "../components/ui/EmptyState";
import { LoadingBlock } from "../components/ui/LoadingBlock";
import { Avatar } from "../components/ui/Avatar";
import { ClassChip } from "../components/ui/ClassChip";
import { computeIssues, type Issue } from "../lib/issues";
import { computeKpis } from "../lib/kpis";
import {
  monthWindow,
  isReadOnlyMonth,
  monthLabel,
  formatTimestamp,
  formatTimeShort,
  WEEKDAYS_SHORT,
} from "../lib/dates";
import type {
  Position,
  Teacher,
  ProposalSummary,
  ProposalDetail,
  EditRow,
  ReviewSuggestion,
  ReviewRunSummary,
  AvailabilityBlock,
  ExternalShiftRow,
} from "../types";

function todayIso(): string {
  return new Date().toISOString().slice(0, 10);
}

const TABS = ["calendar", "list", "edits", "claude"] as const;
type Tab = (typeof TABS)[number];

export function ProposalsScreen({ onGoSettings }: { onGoSettings: () => void }) {
  const today = todayIso();
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
        const next = list[0]?.id ?? null;
        setSelectedId(next);
        if (next == null) setMode("new");
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (selectedId == null) return;
    refreshDetail(selectedId).catch((e) => setError(String(e)));
  }, [selectedId]);

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
        ? computeIssues(detail.shifts, teachers, qualifiedPairs, blocks, externalShifts, [])
        : [],
    [detail, teachers, qualifiedPairs, blocks, externalShifts],
  );

  const kpis = useMemo(() => computeKpis(detail?.shifts ?? []), [detail]);

  // One switcher entry per month (its current proposal), newest month first.
  // The list arrives ordered generated_at DESC, so the first proposal seen
  // per month is the newest; prefer the is_current one as representative.
  const monthEntries = useMemo<MonthEntry[]>(() => {
    const byMonth = new Map<string, { rep: ProposalSummary; count: number }>();
    for (const p of proposals ?? []) {
      const e = byMonth.get(p.target_month);
      if (!e) byMonth.set(p.target_month, { rep: p, count: 1 });
      else {
        e.count += 1;
        if (p.is_current && !e.rep.is_current) e.rep = p;
      }
    }
    return [...byMonth.entries()]
      .sort((a, b) => b[0].localeCompare(a[0]))
      .map(([month, e]) => ({ month, proposalId: e.rep.id, draftCount: e.count }));
  }, [proposals]);

  const selectedSummary =
    selectedId != null ? proposals?.find((p) => p.id === selectedId) : undefined;
  const monthVersions = useMemo(
    () =>
      selectedSummary
        ? (proposals ?? []).filter((p) => p.target_month === selectedSummary.target_month)
        : [],
    [proposals, selectedSummary],
  );

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

  const onGenerate = async () => {
    if (isReadOnlyMonth(activeMonth, today)) return;
    setError(null);
    setLastResult(null);
    setGenerating(true);
    try {
      const result = await api.generateProposal(activeMonth);
      setLastResult(
        `Generated proposal #${result.proposal_id} for ${result.target_month} ` +
          `(${result.algorithm_version}, ${result.shift_count} shifts, ` +
          `${result.dropped_count} dropped)`,
      );
      setSelectedId(result.proposal_id);
      setMode("detail");
      await refreshProposals();
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
    }
  };

  const onProposalChanged = async () => {
    try {
      if (selectedId != null) await refreshDetail(selectedId);
      await refreshProposals();
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
            {mode === "detail" && selectedId != null && monthVersions.length > 0 && (
              <VersionSwitcher
                versions={monthVersions}
                value={selectedId}
                onChange={(id) => setSelectedId(id)}
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
              <button className="btn-ghost" onClick={onGenerate} disabled={generating || readonly} title={readonlyTitle}>
                <Sparkles size={15} /> {generating ? "Generating…" : "Generate"}
              </button>
              <button
                className="btn-primary"
                onClick={() => setPushOpen(true)}
                disabled={readonly}
                title={readonly ? readonlyTitle : "Push these shifts to Sling as planning shifts"}
              >
                <Upload size={15} /> Push to Sling
              </button>
            </>
          ) : undefined
        }
      />

      {pullResult && <div className="ok" style={{ margin: "0 0 14px" }}>{pullResult}</div>}
      {lastResult && <div className="ok" style={{ margin: "0 0 14px" }}>{lastResult}</div>}
      {error && <div className="error" style={{ margin: "0 0 14px" }}>{error}</div>}

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
                  onAction={onGenerate}
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
              onRegenerate={onGenerate}
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
          {tab === "claude" && (
            <>
              <ClaudeEditorPanel
                detail={detail}
                positions={positions}
                teachers={teachers}
                hasKey={hasAnthropicKey}
                onProposalChanged={onProposalChanged}
                onVersionAdopted={() => setAlgoRefresh((n) => n + 1)}
              />
              <ClaudeReviewSection proposalId={detail.summary.id} />
              <AlgorithmCard refreshToken={algoRefresh} />
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
      {pushOpen && detail && (
        <PushModal
          proposalId={detail.summary.id}
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
                {formatTimeShort(s.start_time)}–{formatTimeShort(s.end_time)}
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
              <td className="muted">{formatTimestamp(e.edited_at)}</td>
              <td>
                {e.shift_date} {e.start_time}
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

function ClaudeReviewSection({ proposalId }: { proposalId: number }) {
  const [reviews, setReviews] = useState<ReviewRunSummary[] | null>(null);
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
            {(latest.duration_ms / 1000).toFixed(1)}s · {formatTimestamp(latest.ran_at)}
          </div>
          <p style={{ marginTop: 12 }}>{latest.overall_assessment}</p>
          {latest.suggestions.length === 0 ? (
            <div className="muted">No suggestions — Claude thinks the schedule is fine as-is.</div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
              {latest.suggestions.map((s, i) => (
                <SuggestionCard key={i} s={s} />
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
                  {formatTimestamp(r.ran_at)} · {r.model} · ${r.cost_usd.toFixed(4)}
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

function SuggestionCard({ s }: { s: ReviewSuggestion }) {
  const kindLabel: Record<string, string> = {
    add_rule: "Add rule",
    tweak_parameter: "Tweak parameter",
    fyi: "FYI",
  };
  return (
    <div className="suggestion">
      <div className="row" style={{ marginBottom: 4 }}>
        <span className={`pill pill-${s.type}`}>{kindLabel[s.type] ?? s.type}</span>
        <span className={`pill pill-confidence pill-${s.confidence}`}>{s.confidence}</span>
      </div>
      <div style={{ fontWeight: 600 }}>{s.summary}</div>
      <div className="muted" style={{ fontSize: 13, marginTop: 4 }}>
        {s.rationale}
      </div>
    </div>
  );
}

function weekday(isoDate: string): string {
  const d = new Date(isoDate + "T00:00:00");
  return WEEKDAYS_SHORT[d.getDay()];
}
