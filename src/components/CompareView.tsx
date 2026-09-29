import { useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import { DIFF_KIND_LABEL, defaultComparePair, filterDiff, pct } from "../lib/drafts";
import { formatDayShort, formatTimeShort } from "../lib/dates";
import { LoadingBlock } from "./ui/LoadingBlock";
import type { ProposalDiff, ProposalSummary, TeacherStats } from "../types";

interface Props {
  /** The month's drafts to choose from (newest first). */
  drafts: ProposalSummary[];
  viewingId: number;
  onOpenDraft: (id: number) => void;
}

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

function draftLabel(d: ProposalSummary): string {
  return `${d.name}${d.is_push_candidate ? " (push draft)" : ""}`;
}

/** Side-by-side comparison of two drafts of the same month: changed slots
 *  and per-teacher consistency (distinct weekday+time slots). */
export function CompareView({ drafts, viewingId, onOpenDraft }: Props) {
  const initial = defaultComparePair(drafts, viewingId);
  const [a, setA] = useState<number | null>(initial?.[0] ?? null);
  const [b, setB] = useState<number | null>(initial?.[1] ?? null);
  const [diff, setDiff] = useState<ProposalDiff | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [teacher, setTeacher] = useState("");
  const [weekday, setWeekday] = useState("");

  // Keep the pickers valid when the draft list changes (archive, month switch).
  useEffect(() => {
    const ids = new Set(drafts.map((d) => d.id));
    if (a == null || b == null || !ids.has(a) || !ids.has(b)) {
      const pair = defaultComparePair(drafts, viewingId);
      setA(pair?.[0] ?? null);
      setB(pair?.[1] ?? null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drafts]);

  useEffect(() => {
    if (a == null || b == null) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .diffProposals(a, b)
      .then((d) => !cancelled && setDiff(d))
      .catch((e) => !cancelled && setError(String(e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [a, b, drafts]);

  const teacherNames = useMemo(
    () => (diff ? [...new Set(diff.teachers.map((t) => t.name))].sort() : []),
    [diff],
  );
  const rows = useMemo(
    () => (diff ? filterDiff(diff.changes, { teacher, weekday }) : []),
    [diff, teacher, weekday],
  );

  if (drafts.length < 2 || a == null || b == null) {
    return (
      <div className="card">
        <strong>Compare drafts</strong>
        <div className="muted" style={{ marginTop: 8 }}>
          This month has one draft. Duplicate it (or generate another) from the draft menu next
          to the month title, change the copy, then compare them here.
        </div>
      </div>
    );
  }

  const picker = (value: number, onChange: (id: number) => void, label: string) => (
    <label className="field" style={{ marginBottom: 0, minWidth: 220 }}>
      <span>{label}</span>
      <select value={value} onChange={(e) => onChange(Number(e.target.value))}>
        {drafts.map((d) => (
          <option key={d.id} value={d.id}>
            {draftLabel(d)}
          </option>
        ))}
      </select>
    </label>
  );

  return (
    <>
      <div className="card">
        <div className="row" style={{ alignItems: "flex-end", flexWrap: "wrap" }}>
          {picker(a, setA, "Draft A")}
          {picker(b, setB, "Draft B")}
          <label className="field" style={{ marginBottom: 0 }}>
            <span>Teacher</span>
            <select value={teacher} onChange={(e) => setTeacher(e.target.value)}>
              <option value="">Any teacher</option>
              {teacherNames.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <label className="field" style={{ marginBottom: 0 }}>
            <span>Day</span>
            <select value={weekday} onChange={(e) => setWeekday(e.target.value)}>
              <option value="">Any day</option>
              {WEEKDAYS.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
          </label>
        </div>
        {a === b && <div className="bk-warn">Pick two different drafts.</div>}
        {error && <div className="error">{error}</div>}
      </div>

      {loading && !diff ? (
        <div className="card">
          <LoadingBlock label="Comparing drafts…" />
        </div>
      ) : diff ? (
        <>
          <div className="card">
            <div className="row">
              <strong>
                Changed slots ({rows.length}
                {rows.length !== diff.changes.length && ` of ${diff.changes.length}`})
              </strong>
              <span className="muted" style={{ marginLeft: "auto", fontSize: 12 }}>
                <button className="cell-button" onClick={() => onOpenDraft(diff.a_id)}>
                  Open A
                </button>{" "}
                <button className="cell-button" onClick={() => onOpenDraft(diff.b_id)}>
                  Open B
                </button>
              </span>
            </div>
            {rows.length === 0 ? (
              <div className="muted" style={{ marginTop: 8 }}>
                {diff.changes.length === 0
                  ? "These drafts assign every slot the same way."
                  : "No changed slots match the filter."}
              </div>
            ) : (
              <table style={{ marginTop: 10 }}>
                <thead>
                  <tr>
                    <th>Date</th>
                    <th>Time</th>
                    <th>A: {diff.a_name}</th>
                    <th>B: {diff.b_name}</th>
                    <th>Change</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((c, i) => (
                    <tr key={`${c.date}-${c.start}-${i}`}>
                      <td>{formatDayShort(c.date)}</td>
                      <td>{formatTimeShort(c.start)}</td>
                      <td>{c.class_a ? `${c.class_a} · ${c.teacher_a ?? "—"}` : <span className="muted">—</span>}</td>
                      <td>{c.class_b ? `${c.class_b} · ${c.teacher_b ?? "—"}` : <span className="muted">—</span>}</td>
                      <td className="muted">{DIFF_KIND_LABEL[c.kind] ?? c.kind}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>

          <div className="card">
            <strong>Teacher consistency</strong>
            <div className="muted" style={{ fontSize: 12, marginTop: 4 }}>
              Slots = distinct weekday + start times a teacher teaches this month (fewer = steadier
              week to week). Top slot = her most common one and the share of her classes in it.
            </div>
            <table style={{ marginTop: 10 }}>
              <thead>
                <tr>
                  <th>Teacher</th>
                  <th>A classes</th>
                  <th>A slots</th>
                  <th>A top slot</th>
                  <th>B classes</th>
                  <th>B slots</th>
                  <th>B top slot</th>
                  <th>Δ slots</th>
                </tr>
              </thead>
              <tbody>
                {diff.teachers
                  .filter((t) => !teacher || t.name === teacher)
                  .map((t) => {
                    const delta = (t.b?.distinct_slots ?? 0) - (t.a?.distinct_slots ?? 0);
                    return (
                      <tr key={t.sling_user_id}>
                        <td>{t.name}</td>
                        <StatCells s={t.a} />
                        <StatCells s={t.b} />
                        <td className={delta < 0 ? "bk-better" : delta > 0 ? "bk-worse" : "muted"}>
                          {delta > 0 ? `+${delta}` : delta}
                        </td>
                      </tr>
                    );
                  })}
                <tr className="bk-totals-row">
                  <td>All teachers</td>
                  <td>{diff.totals_a.classes}</td>
                  <td>{diff.totals_a.distinct_slots}</td>
                  <td className="muted">{diff.totals_a.classes_per_slot.toFixed(1)} classes/slot</td>
                  <td>{diff.totals_b.classes}</td>
                  <td>{diff.totals_b.distinct_slots}</td>
                  <td className="muted">{diff.totals_b.classes_per_slot.toFixed(1)} classes/slot</td>
                  <td>
                    {diff.totals_b.distinct_slots - diff.totals_a.distinct_slots > 0 && "+"}
                    {diff.totals_b.distinct_slots - diff.totals_a.distinct_slots}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </>
      ) : null}
    </>
  );
}

function StatCells({ s }: { s: TeacherStats | null }) {
  if (!s) {
    return (
      <>
        <td className="muted">0</td>
        <td className="muted">—</td>
        <td className="muted">—</td>
      </>
    );
  }
  return (
    <>
      <td>{s.classes}</td>
      <td>{s.distinct_slots}</td>
      <td>
        {s.top_slot} <span className="muted">({pct(s.top_slot_share)})</span>
      </td>
    </>
  );
}
