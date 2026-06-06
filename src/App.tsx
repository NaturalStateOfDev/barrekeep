import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./lib/api";
import { CalendarView } from "./components/calendar/CalendarView";
import { SlingTokenModal } from "./components/SlingTokenModal";
import { PushModal } from "./components/PushModal";
import { MonthSelector } from "./components/MonthSelector";
import { UpdateBanner } from "./components/UpdateBanner";
import {
  getCurrentVersion,
  checkForUpdate,
  installUpdate,
  type Update,
  type DownloadProgress,
} from "./lib/updater";
import { monthWindow, isReadOnlyMonth } from "./lib/dates";

function todayIso(): string {
  return new Date().toISOString().slice(0, 10);
}
import type {
  Teacher,
  SlingCandidate,
  Position,
  DbInfo,
  ProposalSummary,
  ProposalDetail,
  EditRow,
  ReviewSuggestion,
  ReviewRunSummary,
  NewUserSummary,
} from "./types";

type View = "dashboard" | "proposals" | "teachers" | "positions" | "settings";

export function App() {
  const [view, setView] = useState<View>("proposals");

  return (
    <div className="app">
      <aside className="sidebar">
        <h1>Barrekeep</h1>
        <button
          className={`nav-item ${view === "dashboard" ? "active" : ""}`}
          onClick={() => setView("dashboard")}
        >
          Dashboard
        </button>
        <button
          className={`nav-item ${view === "proposals" ? "active" : ""}`}
          onClick={() => setView("proposals")}
        >
          Proposals
        </button>
        <button
          className={`nav-item ${view === "teachers" ? "active" : ""}`}
          onClick={() => setView("teachers")}
        >
          Teachers
        </button>
        <button
          className={`nav-item ${view === "positions" ? "active" : ""}`}
          onClick={() => setView("positions")}
        >
          Class types
        </button>
        <button
          className={`nav-item ${view === "settings" ? "active" : ""}`}
          onClick={() => setView("settings")}
          style={{ marginTop: "auto" }}
        >
          Settings
        </button>
      </aside>
      <main className="main">
        <UpdateBanner />
        {view === "dashboard" && <Dashboard />}
        {view === "proposals" && <ProposalsView />}
        {view === "teachers" && <TeachersView />}
        {view === "positions" && <PositionsView />}
        {view === "settings" && <SettingsView />}
      </main>
    </div>
  );
}

function Dashboard() {
  const [info, setInfo] = useState<DbInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.dbInfo().then(setInfo).catch((e) => setError(String(e)));
  }, []);

  return (
    <>
      <h2>Dashboard</h2>
      <div className="card">
        <strong>Database</strong>
        {error && <div className="error">{error}</div>}
        {info && (
          <table>
            <tbody>
              <tr>
                <td className="muted">Path</td>
                <td>
                  <code>{info.path}</code>
                </td>
              </tr>
              <tr>
                <td className="muted">Schema version</td>
                <td>{info.schema_version}</td>
              </tr>
              <tr>
                <td className="muted">Teachers</td>
                <td>{info.teacher_count}</td>
              </tr>
              <tr>
                <td className="muted">Class types</td>
                <td>{info.position_count}</td>
              </tr>
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

function ProposalsView() {
  const [proposals, setProposals] = useState<ProposalSummary[] | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<ProposalDetail | null>(null);
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastResult, setLastResult] = useState<string | null>(null);
  const [tab, setTab] = useState<"calendar" | "list" | "edits" | "review">("calendar");
  const [slingExpiredModal, setSlingExpiredModal] = useState(false);
  const [pushOpen, setPushOpen] = useState(false);

  const today = todayIso();
  const [selectedMonth, setSelectedMonth] = useState<string>(() => {
    // Default to next month per the spec; refined in Task 13.
    const [y, m] = today.split("-").map(Number);
    const ny = m === 12 ? y + 1 : y;
    const nm = m === 12 ? 1 : m + 1;
    return `${ny}-${String(nm).padStart(2, "0")}`;
  });
  const [pulling, setPulling] = useState(false);
  const [pullResult, setPullResult] = useState<string | null>(null);
  const [newUsersFromPull, setNewUsersFromPull] = useState<NewUserSummary[]>([]);

  const refreshProposals = async () => {
    const list = await api.listProposals();
    setProposals(list);
    return list;
  };

  const refreshDetail = async (id: number) => {
    const d = await api.getProposal(id);
    setDetail(d);
  };

  useEffect(() => {
    refreshProposals()
      .then((list) => {
        const next = list[0]?.id ?? null;
        setSelectedId(next);
        if (next != null) refreshDetail(next);
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (selectedId == null) return;
    refreshDetail(selectedId).catch((e) => setError(String(e)));
  }, [selectedId]);

  useEffect(() => {
    setNewUsersFromPull([]);
    setPullResult(null);
  }, [selectedMonth]);

  // Default month per spec: first empty in priority next > next+1 > current > previous.
  // Only fires on the FIRST proposals load — subsequent refreshes (after a
  // pull or generate) must not override an explicit user selection.
  const defaultMonthPicked = useRef(false);
  useEffect(() => {
    if (!proposals || defaultMonthPicked.current) return;
    defaultMonthPicked.current = true;
    const window = monthWindow(today);
    // window = [prev, current, next, next+1]
    const priority = [window[2], window[3], window[1], window[0]];
    const proposedMonths = new Set(proposals.map((p) => p.target_month));
    const firstEmpty = priority.find((m) => !proposedMonths.has(m));
    if (firstEmpty) setSelectedMonth(firstEmpty);
  }, [proposals]);

  const onPull = async () => {
    setError(null);
    setPullResult(null);
    setPulling(true);
    try {
      const r = await api.pullMonthFromSling(selectedMonth);
      setPullResult(
        `Pulled ${selectedMonth}: ${r.user_count} users, ${r.qual_count} qualifications, ` +
          `${r.availability_count} availability blocks, ${r.external_shift_count} external shifts, ` +
          `${r.history_shift_count} trailing-history shifts.` +
          (r.new_users.length ? ` ${r.new_users.length} new teacher(s) detected.` : ""),
      );
      setNewUsersFromPull(r.new_users);
      await refreshProposals();
    } catch (e) {
      const msg = String(e);
      if (msg.includes("sling-401")) {
        setSlingExpiredModal(true);
      } else {
        setError(msg);
      }
    } finally {
      setPulling(false);
    }
  };

  const onGenerate = async () => {
    setError(null);
    setLastResult(null);
    setGenerating(true);
    try {
      const result = await api.generateProposal(selectedMonth);
      setLastResult(
        `Generated proposal #${result.proposal_id} for ${result.target_month} ` +
          `(${result.algorithm_version}, ${result.shift_count} shifts, ` +
          `${result.dropped_count} dropped)`,
      );
      setSelectedId(result.proposal_id);
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

  return (
    <>
      <h2>Proposals</h2>

      <div className="card">
        <div className="row">
          <span>Month:</span>
          <MonthSelector
            today={today}
            value={selectedMonth}
            onChange={setSelectedMonth}
          />
        </div>
        <div className="row" style={{ marginTop: 12 }}>
          <button
            className="btn-ghost"
            onClick={onPull}
            disabled={pulling || isReadOnlyMonth(selectedMonth, today)}
            title={isReadOnlyMonth(selectedMonth, today) ? "Past month — read only" : ""}
          >
            {pulling ? "Pulling…" : `Pull from Sling for ${selectedMonth}`}
          </button>
          <button
            className="btn-primary"
            onClick={onGenerate}
            disabled={generating || isReadOnlyMonth(selectedMonth, today)}
            title={isReadOnlyMonth(selectedMonth, today) ? "Past month — read only" : ""}
          >
            {generating ? "Generating…" : `Generate proposal for ${selectedMonth}`}
          </button>
        </div>
        {pullResult && <div className="ok">{pullResult}</div>}
        {lastResult && <div className="ok">{lastResult}</div>}
        {error && <div className="error">{error}</div>}
      </div>

      {proposals && proposals.length > 0 && (
        <div className="card">
          <strong>History</strong>
          <table>
            <thead>
              <tr>
                <th>ID</th>
                <th>Month</th>
                <th>Algo</th>
                <th>Generated</th>
                <th>Shifts</th>
                <th>Dropped</th>
                <th>Edits</th>
                <th>Current</th>
              </tr>
            </thead>
            <tbody>
              {proposals.map((p) => (
                <tr
                  key={p.id}
                  className={p.id === selectedId ? "selected" : ""}
                  onClick={() => setSelectedId(p.id)}
                  style={{ cursor: "pointer" }}
                >
                  <td>{p.id}</td>
                  <td>{p.target_month}</td>
                  <td>{p.algorithm_version}</td>
                  <td className="muted">{formatTimestamp(p.generated_at)}</td>
                  <td>{p.shift_count}</td>
                  <td>{p.dropped_count}</td>
                  <td>{p.edit_count > 0 ? <strong>{p.edit_count}</strong> : 0}</td>
                  <td>{p.is_current ? "yes" : ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {detail && (
        <>
          <div className="row" style={{ justifyContent: "flex-end", marginBottom: 8 }}>
            <button
              className="btn-primary"
              onClick={() => setPushOpen(true)}
              disabled={isReadOnlyMonth(detail.summary.target_month, today)}
              title={isReadOnlyMonth(detail.summary.target_month, today) ? "Past month — read only" : "Push these shifts to Sling as planning shifts"}
            >
              Push to Sling
            </button>
          </div>
          <div className="bk-tabs">
            <button onClick={() => setTab("calendar")} className={tab === "calendar" ? "active" : ""}>Calendar</button>
            <button onClick={() => setTab("list")} className={tab === "list" ? "active" : ""}>List</button>
            <button onClick={() => setTab("edits")} className={tab === "edits" ? "active" : ""}>Edits</button>
            <button onClick={() => setTab("review")} className={tab === "review" ? "active" : ""}>Review</button>
          </div>
          {tab === "calendar" && (
            <CalendarView
              proposal={detail}
              newUsersFromPull={newUsersFromPull}
              onProposalChanged={onProposalChanged}
              onRegenerate={onGenerate}
              readonly={isReadOnlyMonth(detail.summary.target_month, today)}
            />
          )}
          {tab === "list" && <ProposalShiftsTable detail={detail} />}
          {tab === "edits" && <EditHistory proposalId={detail.summary.id} />}
          {tab === "review" && <ClaudeReviewSection proposalId={detail.summary.id} />}
        </>
      )}
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
          monthLabel={detail.summary.target_month}
          onClose={() => { setPushOpen(false); onProposalChanged(); }}
          onTokenExpired={() => { setPushOpen(false); setSlingExpiredModal(true); }}
        />
      )}
    </>
  );
}

function ProposalShiftsTable({
  detail,
}: {
  detail: ProposalDetail;
}) {
  const { summary, shifts } = detail;
  return (
    <div className="card">
      <div className="row">
        <strong>
          Proposal #{summary.id} — {summary.target_month} ({summary.algorithm_version})
        </strong>
        {summary.edit_count > 0 && (
          <span className="badge">{summary.edit_count} manual edit{summary.edit_count === 1 ? "" : "s"}</span>
        )}
        <span className="muted" style={{ marginLeft: "auto", fontSize: 12 }}>
          Read-only view. Edit teachers from the Calendar tab.
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
                {s.start_time}–{s.end_time}
              </td>
              <td>{s.class_name}</td>
              <td>
                {s.is_coteach ? (
                  <strong>{s.coteach_label}</strong>
                ) : s.is_dropped ? (
                  <span className="muted">DROPPED</span>
                ) : (
                  s.teacher_name ?? <span className="muted">—</span>
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
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .listEditsForProposal(proposalId)
      .then(setEdits)
      .catch((e) => setError(String(e)));
  }, [proposalId]);

  if (error) return <div className="card error">{error}</div>;
  if (!edits || edits.length === 0) return null;

  return (
    <div className="card">
      <button className="disclosure" onClick={() => setOpen(!open)}>
        {open ? "▾" : "▸"} Edit history ({edits.length})
      </button>
      {open && (
        <table>
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
                <td>{e.old_teacher_name ?? <span className="muted">DROPPED</span>}</td>
                <td>{e.new_teacher_name ?? <span className="muted">DROPPED</span>}</td>
                <td className="muted">{e.reason ?? ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function TeachersView() {
  const [teachers, setTeachers] = useState<Teacher[] | null>(null);
  const [candidates, setCandidates] = useState<SlingCandidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => {
    api.listTeachers().then(setTeachers).catch((e) => setError(String(e)));
    api.listSlingCandidates().then(setCandidates).catch((e) => setError(String(e)));
  };

  useEffect(() => { refresh(); }, []);

  // Candidates that aren't already in the teachers roster.
  const addable = useMemo(() => {
    if (!candidates || !teachers) return [];
    const have = new Set(teachers.map((t) => t.sling_user_id));
    return candidates.filter((c) => !have.has(c.sling_user_id));
  }, [candidates, teachers]);

  return (
    <>
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        <h2 style={{ margin: 0 }}>Teachers</h2>
        <span
          title="Studio managed by this app (read-only for now)"
          style={{
            background: "hsl(210 50% 94%)",
            color: "hsl(210 60% 32%)",
            border: "1px solid hsl(210 40% 80%)",
            borderRadius: 999,
            padding: "2px 10px",
            fontSize: 13,
            fontWeight: 600,
          }}
        >
          Studio: the studio
        </span>
      </div>
      {error && <div className="card error">{error}</div>}
      {teachers && (
        <div className="card">
          <table>
            <thead>
              <tr>
                <th>Name</th>
                <th>Sling ID</th>
                <th>Location</th>
                <th>Target/wk</th>
                <th>Max/wk</th>
                <th>Lead</th>
                <th>Active</th>
              </tr>
            </thead>
            <tbody>
              {teachers.map((t) => (
                <TeacherRow
                  key={t.sling_user_id}
                  teacher={t}
                  onSaved={refresh}
                  onError={setError}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
      <AddTeacherCard candidates={addable} onAdded={refresh} />
    </>
  );
}

function TeacherRow({
  teacher,
  onSaved,
  onError,
}: {
  teacher: Teacher;
  onSaved: () => void;
  onError: (msg: string) => void;
}) {
  const [target, setTarget] = useState(String(teacher.weekly_target));
  const [max, setMax] = useState(String(teacher.weekly_max));
  const [saving, setSaving] = useState(false);

  // If the teacher row re-fetches (e.g. after a pull), pick up the new values.
  useEffect(() => { setTarget(String(teacher.weekly_target)); }, [teacher.weekly_target]);
  useEffect(() => { setMax(String(teacher.weekly_max)); }, [teacher.weekly_max]);

  const commit = async () => {
    const t = Number(target);
    const m = Number(max);
    if (!Number.isFinite(t) || !Number.isFinite(m) || t < 0 || m < 0) {
      setTarget(String(teacher.weekly_target));
      setMax(String(teacher.weekly_max));
      onError("Target and max must be non-negative numbers.");
      return;
    }
    if (t === teacher.weekly_target && m === teacher.weekly_max) return;
    setSaving(true);
    try {
      await api.updateTeacherSettings(teacher.sling_user_id, t, m);
      onSaved();
    } catch (e) {
      onError(String(e));
      setTarget(String(teacher.weekly_target));
      setMax(String(teacher.weekly_max));
    } finally {
      setSaving(false);
    }
  };

  const inputStyle = {
    width: 56,
    padding: "2px 4px",
    border: "1px solid transparent",
    borderRadius: 4,
    background: "transparent",
  };

  return (
    <tr>
      <td>{teacher.display_name}</td>
      <td className="muted"><code>{teacher.sling_user_id}</code></td>
      <td>{teacher.locations || <span className="muted">—</span>}</td>
      <td>
        <input
          type="number"
          min={0}
          value={target}
          disabled={saving}
          onChange={(e) => setTarget(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
            if (e.key === "Escape") {
              setTarget(String(teacher.weekly_target));
              (e.target as HTMLInputElement).blur();
            }
          }}
          onFocus={(e) => {
            e.currentTarget.style.border = "1px solid #aab";
            e.currentTarget.style.background = "#f6f8fa";
          }}
          onBlurCapture={(e) => {
            e.currentTarget.style.border = "1px solid transparent";
            e.currentTarget.style.background = "transparent";
          }}
          style={inputStyle}
        />
      </td>
      <td>
        <input
          type="number"
          min={0}
          value={max}
          disabled={saving}
          onChange={(e) => setMax(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
            if (e.key === "Escape") {
              setMax(String(teacher.weekly_max));
              (e.target as HTMLInputElement).blur();
            }
          }}
          onFocus={(e) => {
            e.currentTarget.style.border = "1px solid #aab";
            e.currentTarget.style.background = "#f6f8fa";
          }}
          onBlurCapture={(e) => {
            e.currentTarget.style.border = "1px solid transparent";
            e.currentTarget.style.background = "transparent";
          }}
          style={inputStyle}
        />
      </td>
      <td>{teacher.is_lead ? "yes" : ""}</td>
      <td>{teacher.active ? "yes" : "no"}</td>
    </tr>
  );
}

function AddTeacherCard({
  candidates,
  onAdded,
}: {
  candidates: SlingCandidate[];
  onAdded: () => void;
}) {
  const [selected, setSelected] = useState<number | "">("");
  const [weeklyTarget, setWeeklyTarget] = useState(4);
  const [weeklyMax, setWeeklyMax] = useState(5);
  const [isLead, setIsLead] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const onAdd = async () => {
    setStatus(null);
    setError(null);
    if (selected === "") {
      setError("Pick a teacher from the list.");
      return;
    }
    const cand = candidates.find((c) => c.sling_user_id === selected);
    if (!cand) {
      setError("Selected teacher no longer in candidates list.");
      return;
    }
    try {
      await api.addTeacherFromPull({
        sling_user_id: cand.sling_user_id,
        display_name: cand.display_name,
        weekly_target: weeklyTarget,
        weekly_max: weeklyMax,
        is_lead: isLead,
      });
      setStatus(`Added ${cand.display_name}.`);
      setSelected("");
      onAdded();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="card" style={{ marginTop: 16 }}>
      <strong>Add a teacher from Sling</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Adds them to this app's scheduling roster only. Doesn't touch Sling.
        The list refreshes when you pull from Sling.
      </p>
      {candidates.length === 0 ? (
        <div className="muted" style={{ marginTop: 8 }}>
          No add-able candidates. Pull from Sling on the Proposals page to refresh.
        </div>
      ) : (
        <>
          <div style={{ marginTop: 12, display: "flex", flexDirection: "column", gap: 8, maxWidth: 480 }}>
            <select
              value={selected}
              onChange={(e) => setSelected(e.target.value === "" ? "" : Number(e.target.value))}
              style={{ padding: "6px 8px" }}
            >
              <option value="">— pick a teacher —</option>
              {candidates.map((c) => (
                <option key={c.sling_user_id} value={c.sling_user_id}>
                  {c.display_name}
                  {c.locations ? ` (${c.locations})` : ""}
                </option>
              ))}
            </select>
            <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <label style={{ display: "flex", flexDirection: "column", fontSize: 12 }}>
                <span className="muted">Target/wk</span>
                <input
                  type="number"
                  min={0}
                  max={20}
                  value={weeklyTarget}
                  onChange={(e) => setWeeklyTarget(Number(e.target.value))}
                  style={{ padding: "6px 8px", width: 80 }}
                />
              </label>
              <label style={{ display: "flex", flexDirection: "column", fontSize: 12 }}>
                <span className="muted">Max/wk</span>
                <input
                  type="number"
                  min={0}
                  max={20}
                  value={weeklyMax}
                  onChange={(e) => setWeeklyMax(Number(e.target.value))}
                  style={{ padding: "6px 8px", width: 80 }}
                />
              </label>
              <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <input
                  type="checkbox"
                  checked={isLead}
                  onChange={(e) => setIsLead(e.target.checked)}
                />
                <span>Lead</span>
              </label>
            </div>
          </div>
          <div className="row" style={{ marginTop: 12 }}>
            <button className="btn-primary" onClick={onAdd}>Add to roster</button>
          </div>
        </>
      )}
      {status && <div className="ok" style={{ marginTop: 8 }}>{status}</div>}
      {error && <div className="error" style={{ marginTop: 8 }}>{error}</div>}
    </div>
  );
}

function PositionsView() {
  const [positions, setPositions] = useState<Position[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.listPositions().then(setPositions).catch((e) => setError(String(e)));
  }, []);

  return (
    <>
      <h2>Class types</h2>
      {error && <div className="card error">{error}</div>}
      {positions && (
        <div className="card">
          <table>
            <thead>
              <tr>
                <th>Class</th>
                <th>Sling position ID</th>
                <th>Duration (min)</th>
                <th>Special</th>
              </tr>
            </thead>
            <tbody>
              {positions.map((p) => (
                <tr key={p.sling_position_id}>
                  <td>{p.class_name}</td>
                  <td className="muted">
                    <code>{p.sling_position_id}</code>
                  </td>
                  <td>{p.duration_minutes}</td>
                  <td>{p.is_special ? "yes" : ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function SettingsView() {
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.hasAnthropicKey().then(setHasKey).catch((e) => setError(String(e)));
  }, []);

  const onSave = async () => {
    setError(null);
    setStatus(null);
    try {
      await api.setAnthropicKey(keyInput);
      setKeyInput("");
      const has = await api.hasAnthropicKey();
      setHasKey(has);
      setStatus(has ? "Saved. The key is held in memory only — paste again next session." : "Cleared.");
    } catch (e) {
      setError(String(e));
    }
  };

  const onClear = async () => {
    try {
      await api.setAnthropicKey("");
      setHasKey(false);
      setStatus("Cleared.");
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <>
      <h2>Settings</h2>

      <StudioConfigCard />

      <div className="card">
        <strong>Anthropic API key</strong>
        <p className="muted" style={{ marginTop: 4 }}>
          Required for the "Have Claude review" feature on proposals. Stored
          in memory only — closes with the app, paste again next session.
          Get a key from <code>console.anthropic.com</code>.
        </p>
        <div style={{ marginTop: 12 }}>
          Status:{" "}
          {hasKey === null ? (
            <span className="muted">checking…</span>
          ) : hasKey ? (
            <span style={{ color: "#1a7f37", fontWeight: 600 }}>set</span>
          ) : (
            <span className="muted">not set</span>
          )}
        </div>
        <label className="field" style={{ marginTop: 12 }}>
          <span>Paste key</span>
          <input
            type="password"
            value={keyInput}
            onChange={(e) => setKeyInput(e.target.value)}
            placeholder="sk-ant-..."
            style={{
              width: "100%",
              padding: "8px",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius)",
              fontFamily: "var(--font-mono)",
            }}
          />
        </label>
        <div className="row" style={{ marginTop: 12 }}>
          <button className="btn-primary" onClick={onSave} disabled={!keyInput}>
            Save
          </button>
          {hasKey && (
            <button className="btn-ghost" onClick={onClear}>
              Clear
            </button>
          )}
        </div>
        {status && <div className="ok">{status}</div>}
        {error && <div className="error">{error}</div>}
      </div>

      <SlingTokenCard />

      <SlingCredentialsCard />

      <UpdatesCard />
    </>
  );
}

function UpdatesCard() {
  const [version, setVersion] = useState<string>("");
  const [update, setUpdate] = useState<Update | null>(null);
  const [state, setState] = useState<
    "idle" | "checking" | "current" | "available" | "installing" | "error"
  >("idle");
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getCurrentVersion().then(setVersion).catch(() => {});
  }, []);

  const onCheck = async () => {
    setState("checking");
    setError(null);
    try {
      const u = await checkForUpdate();
      if (u) {
        setUpdate(u);
        setState("available");
      } else {
        setState("current");
      }
    } catch (e) {
      setState("error");
      setError(String(e));
    }
  };

  const onInstall = async () => {
    if (!update) return;
    setState("installing");
    setError(null);
    try {
      await installUpdate(update, setProgress);
      // relaunches on success
    } catch (e) {
      setState("error");
      setError(String(e));
    }
  };

  const pct = progress?.percent;

  return (
    <div className="card">
      <strong>Updates</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Barrekeep checks for a newer signed release on startup and installs it
        with your approval. You can also check on demand here.
      </p>
      <div style={{ marginTop: 12 }}>
        Current version:{" "}
        {version ? <code>v{version}</code> : <span className="muted">…</span>}
      </div>

      <div className="row" style={{ marginTop: 12 }}>
        {state === "available" ? (
          <button className="btn-primary" onClick={onInstall}>
            Install v{update?.version} &amp; restart
          </button>
        ) : (
          <button
            className="btn-primary"
            onClick={onCheck}
            disabled={state === "checking" || state === "installing"}
          >
            {state === "checking" ? "Checking…" : "Check for updates"}
          </button>
        )}
      </div>

      {state === "current" && (
        <div className="ok" style={{ marginTop: 8 }}>You're on the latest version.</div>
      )}
      {state === "available" && (
        <div className="ok" style={{ marginTop: 8 }}>
          v{update?.version} is ready to install.
        </div>
      )}
      {state === "installing" && (
        <div className="muted" style={{ marginTop: 8 }}>
          Downloading{pct != null ? ` ${pct}%` : "…"} — the app will restart when done.
        </div>
      )}
      {state === "error" && (
        <div className="error" style={{ marginTop: 8 }}>
          Couldn't check for updates: {error}
        </div>
      )}
    </div>
  );
}

function StudioConfigCard() {
  const [orgId, setOrgId] = useState("");
  const [actingUserId, setActingUserId] = useState("");
  const [homeLocationId, setHomeLocationId] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    api.getStudioConfig().then((c) => {
      setOrgId(String(c.org_id));
      setActingUserId(String(c.acting_user_id));
      setHomeLocationId(String(c.home_location_id));
      setLoaded(true);
    }).catch((e) => setError(String(e)));

  useEffect(() => { refresh(); }, []);

  const configured = loaded && Number(orgId) > 0 && Number(homeLocationId) > 0;

  const onSave = async () => {
    setError(null);
    setStatus(null);
    const o = Number(orgId), a = Number(actingUserId), h = Number(homeLocationId);
    if (![o, a, h].every((n) => Number.isInteger(n) && n >= 0)) {
      setError("All three IDs must be non-negative whole numbers.");
      return;
    }
    try {
      await api.setStudioConfig(o, a, h);
      setStatus("Saved. Pulls will now target this studio.");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const fieldStyle = {
    width: "100%", padding: "8px",
    border: "1px solid var(--color-border)", borderRadius: "var(--radius)",
    fontFamily: "var(--font-mono)",
  };

  return (
    <div className="card">
      <strong>Studio configuration</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Your studio's Sling identifiers. Required before pulling. Find them in a
        Sling DevTools session: the <code>org id</code> and admin{" "}
        <code>acting-user id</code> appear in the calendar request URL, and the{" "}
        <code>home location id</code> is your studio's location (other locations
        are filtered out). Stored locally in this app's database only.
      </p>
      <div style={{ marginTop: 12 }}>
        Status:{" "}
        {!loaded ? <span className="muted">checking…</span>
          : configured ? <span style={{ color: "#1a7f37", fontWeight: 600 }}>configured</span>
          : <span style={{ color: "#b54708", fontWeight: 600 }}>not configured — pulls disabled</span>}
      </div>
      <label className="field" style={{ marginTop: 12 }}>
        <span>Organization id</span>
        <input type="number" min={0} value={orgId} onChange={(e) => setOrgId(e.target.value)} placeholder="0" style={fieldStyle} />
      </label>
      <label className="field" style={{ marginTop: 8 }}>
        <span>Acting-user id (admin calendar feed)</span>
        <input type="number" min={0} value={actingUserId} onChange={(e) => setActingUserId(e.target.value)} placeholder="0" style={fieldStyle} />
      </label>
      <label className="field" style={{ marginTop: 8 }}>
        <span>Home location id</span>
        <input type="number" min={0} value={homeLocationId} onChange={(e) => setHomeLocationId(e.target.value)} placeholder="0" style={fieldStyle} />
      </label>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={onSave}>Save</button>
      </div>
      {status && <div className="ok">{status}</div>}
      {error && <div className="error">{error}</div>}
    </div>
  );
}

function SlingCredentialsCard() {
  const [hasCreds, setHasCreds] = useState<boolean | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    api.hasSlingCredentials().then(setHasCreds).catch((e) => setError(String(e)));

  useEffect(() => { refresh(); }, []);

  const onSave = async () => {
    setError(null);
    setStatus(null);
    if (!email.trim()) {
      setError("Email is required.");
      return;
    }
    try {
      await api.setSlingCredentials(email.trim(), password);
      setEmail("");
      setPassword("");
      setStatus("Saved. Sling login form will be pre-filled next time.");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onClear = async () => {
    setError(null);
    setStatus(null);
    try {
      await api.setSlingCredentials("", "");
      setStatus("Cleared.");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="card">
      <strong>Sling login credentials (optional)</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Saved in OS keychain (Stronghold) and used only to pre-fill Sling's
        login form when you click "Log in via Sling". Captcha and the submit
        click stay with you. Leave blank if you'd rather type them each time.
      </p>
      <div style={{ marginTop: 12 }}>
        Status:{" "}
        {hasCreds === null ? <span className="muted">checking…</span>
          : hasCreds ? <span style={{ color: "#1a7f37", fontWeight: 600 }}>saved</span>
          : <span className="muted">not saved</span>}
      </div>
      <div style={{ marginTop: 12, display: "flex", flexDirection: "column", gap: 8, maxWidth: 360 }}>
        <input
          type="email"
          autoComplete="off"
          placeholder="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          style={{ padding: "6px 8px" }}
        />
        <input
          type="password"
          autoComplete="new-password"
          placeholder="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          style={{ padding: "6px 8px" }}
        />
      </div>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={onSave}>
          {hasCreds ? "Update" : "Save"}
        </button>
        {hasCreds && (
          <button className="btn-ghost" onClick={onClear}>Clear</button>
        )}
      </div>
      {status && <div className="ok">{status}</div>}
      {error && <div className="error">{error}</div>}
    </div>
  );
}

function SlingTokenCard() {
  const [hasToken, setHasToken] = useState<boolean | null>(null);
  const [showModal, setShowModal] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => api.hasSlingToken().then(setHasToken).catch((e) => setError(String(e)));

  useEffect(() => { refresh(); }, []);

  const [toast, setToast] = useState<string | null>(null);

  useEffect(() => {
    const unsubs: Array<Promise<() => void>> = [];
    unsubs.push(listen<void>("sling-token-saved", () => {
      setToast("Logged in to Sling.");
      refresh();
    }));
    unsubs.push(listen<void>("sling-login-cancelled", () => {
      setToast("Sign-in cancelled.");
    }));
    return () => {
      unsubs.forEach((p) => p.then((u) => u()));
    };
  }, []);

  const onLoginBrowser = async () => {
    setError(null);
    setToast(null);
    try {
      await api.openSlingLoginWindow();
    } catch (e) {
      setError(String(e));
    }
  };

  const onClear = async () => {
    try {
      await api.setSlingToken("");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="card">
      <strong>Sling token</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Required for "Pull from Sling". Stored in OS keychain (Stronghold); survives
        app restarts. If a pull returns 401, you'll be prompted to paste a fresh one.
      </p>
      <div style={{ marginTop: 12 }}>
        Status:{" "}
        {hasToken === null ? <span className="muted">checking…</span>
          : hasToken ? <span style={{ color: "#1a7f37", fontWeight: 600 }}>set</span>
          : <span className="muted">not set</span>}
      </div>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={() => setShowModal(true)}>
          {hasToken ? "Update" : "Set token"}
        </button>
        <button className="btn-ghost" onClick={onLoginBrowser}>
          Log in via Sling
        </button>
        {hasToken && <button className="btn-ghost" onClick={onClear}>Clear</button>}
      </div>
      {error && <div className="error">{error}</div>}
      {toast && <div className="ok">{toast}</div>}
      {showModal && (
        <SlingTokenModal
          reason="first-time"
          onSaved={() => { setShowModal(false); refresh(); }}
          onCancel={() => setShowModal(false)}
        />
      )}
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
          title={hasKey ? "" : "Set your API key on the Settings tab first"}
        >
          {running ? "Reviewing…" : latest ? "Run again" : "Have Claude review"}
        </button>
      </div>
      {!hasKey && (
        <div className="muted" style={{ marginTop: 8 }}>
          Set your Anthropic API key on the Settings tab to enable this.
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
              <div key={r.id} className="muted" style={{ fontSize: 12, paddingLeft: 12, borderLeft: "2px solid var(--color-border)" }}>
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

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
function weekday(isoDate: string): string {
  const d = new Date(isoDate + "T00:00:00");
  return WEEKDAYS[d.getDay()];
}

function formatTimestamp(iso: string): string {
  // DuckDB returns 'YYYY-MM-DD HH:MM:SS+TZ'. Trim to local-ish display.
  return iso.replace("T", " ").replace(/\.\d+/, "").slice(0, 19);
}
