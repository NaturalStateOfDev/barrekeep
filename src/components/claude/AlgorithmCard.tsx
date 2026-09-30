import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../lib/api";
import { ruleLines } from "../../lib/rules";
import { useTimeFormat } from "../../lib/timeFormat";
import type { AlgorithmVersion, Teacher } from "../../types";

interface Props {
  /** Bump to refetch after an adoption elsewhere on the page. */
  refreshToken: number;
  teachers: Teacher[];
}

const BASELINE = 9;

function scriptBadge(v: AlgorithmVersion): string {
  if (!v.script_file) return "baseline script";
  if (v.script_missing) return "script deleted";
  if (v.script_archived) return "script archived";
  return v.script_file;
}

/** Active algorithm version + adoption history: "Make active" rolls back or
 *  forward (including to the v9 baseline), plus manual script deletion. */
export function AlgorithmCard({ refreshToken, teachers }: Props) {
  const [versions, setVersions] = useState<AlgorithmVersion[] | null>(null);
  const tf = useTimeFormat();
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const [busy, setBusy] = useState<number | null>(null);
  const inFlight = useRef(false);

  const teacherName = useMemo(() => {
    const byId = new Map(teachers.map((t) => [String(t.sling_user_id), t.display_name]));
    return (uid: unknown) => byId.get(String(uid)) ?? `Teacher ${uid}`;
  }, [teachers]);

  const refresh = () =>
    api.listAlgorithmVersions().then(setVersions).catch((e) => setError(String(e)));

  useEffect(() => {
    refresh();
  }, [refreshToken]);

  const active = versions?.find((v) => v.is_active) ?? null;
  const activeNumber = active ? active.version : BASELINE;

  const guarded = async (version: number, fn: () => Promise<void>) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(version);
    setError(null);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      inFlight.current = false;
      setBusy(null);
    }
  };

  const onMakeActive = (version: number) =>
    guarded(version, () => api.setActiveAlgorithmVersion(version));

  const onDelete = (version: number) => {
    if (confirmDelete !== version) {
      setConfirmDelete(version);
      return;
    }
    setConfirmDelete(null);
    guarded(version, () => api.deleteAlgorithmScript(version));
  };

  const makeActiveButton = (version: number, disabled = false) => (
    <button
      className="btn-ghost btn-sm"
      disabled={busy != null || disabled}
      onClick={() => onMakeActive(version)}
      title={disabled ? "Its script was deleted" : "Use this version for the next Generate"}
    >
      {busy === version ? "Switching…" : "Make active"}
    </button>
  );

  const lines = active ? ruleLines(active.rules, teacherName, tf.fmt) : [];

  return (
    <div className="card">
      <strong>Algorithm</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        The rule set and script the next Generate will use. Versions are adopted from
        Claude's proposals (or your own) and never change once adopted; make an older one
        active to roll back.
      </p>
      <div style={{ marginTop: 10 }}>
        Active: <strong>v{activeNumber}</strong>
        {active ? (
          <>
            {" — "}
            {active.description}
            <span className="muted">
              {" · adopted "}
              {active.adopted_at.slice(0, 10)}
              {active.last_used_month && ` · last used ${active.last_used_month}`}
            </span>
          </>
        ) : (
          <span className="muted"> — the shipped baseline, no standing rules.</span>
        )}
      </div>
      {active?.baseline_outdated && (
        <div className="bk-warn">
          v{active.version} runs a custom script based on an older baseline — this app update
          ships a newer propose.py, and its changes are not in v{active.version}'s script.
          Re-draft the code change (or make the baseline active) to pick them up.
        </div>
      )}
      {active && (
        <div style={{ marginTop: 8 }}>
          <button className="disclosure" onClick={() => setExpanded(!expanded)}>
            {expanded ? "Hide rules" : "Show rules"}
          </button>
          {expanded && (
            <ul style={{ margin: "4px 0 0", paddingLeft: 20 }}>
              {lines.map((line, i) => (
                <li key={i} style={{ fontSize: 13 }}>{line}</li>
              ))}
              {lines.length === 0 && (
                <li className="muted" style={{ fontSize: 13 }}>No standing rules.</li>
              )}
            </ul>
          )}
        </div>
      )}
      {versions && versions.length > 0 && (
        <table style={{ marginTop: 12 }}>
          <thead>
            <tr>
              <th>Version</th>
              <th>Description</th>
              <th>By</th>
              <th>Adopted</th>
              <th>Last used</th>
              <th>Script</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {versions.map((v) => (
              <tr key={v.version}>
                <td>
                  v{v.version}
                  {v.is_active && <span className="badge" style={{ marginLeft: 6 }}>Active</span>}
                </td>
                <td>{v.description}</td>
                <td className="muted">{v.created_by}</td>
                <td className="muted">{v.adopted_at.slice(0, 10)}</td>
                <td className="muted">{v.last_used_month ?? "—"}</td>
                <td className="muted">
                  <code style={{ fontSize: 11 }}>{scriptBadge(v)}</code>
                  {v.baseline_outdated && (
                    <span
                      className="badge bk-badge-warn"
                      style={{ marginLeft: 6 }}
                      title="This script was built on an older shipped propose.py"
                    >
                      older baseline
                    </span>
                  )}
                </td>
                <td>
                  <div className="row" style={{ gap: 6, flexWrap: "nowrap" }}>
                    {!v.is_active && makeActiveButton(v.version, v.script_missing)}
                    {v.script_file &&
                      !v.script_missing &&
                      !v.is_active &&
                      v.script_file !== active?.script_file && (
                        <button
                          className="btn-ghost btn-sm"
                          disabled={busy != null}
                          onClick={() => onDelete(v.version)}
                        >
                          {confirmDelete === v.version ? "Really delete?" : "Delete script"}
                        </button>
                      )}
                  </div>
                </td>
              </tr>
            ))}
            <tr>
              <td>
                v{BASELINE}
                {activeNumber === BASELINE && (
                  <span className="badge" style={{ marginLeft: 6 }}>Active</span>
                )}
              </td>
              <td>Shipped baseline — no standing rules</td>
              <td className="muted">app</td>
              <td className="muted">—</td>
              <td className="muted">—</td>
              <td className="muted"><code style={{ fontSize: 11 }}>baseline script</code></td>
              <td>{activeNumber !== BASELINE && makeActiveButton(BASELINE)}</td>
            </tr>
          </tbody>
        </table>
      )}
      {error && <div className="error">{error}</div>}
    </div>
  );
}
