import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import type { AlgorithmVersion } from "../../types";

interface Props {
  /** Bump to refetch after an adoption elsewhere on the page. */
  refreshToken: number;
}

function ruleLines(rules: Record<string, unknown>): string[] {
  const out: string[] = [];
  const arr = (k: string) => (Array.isArray(rules[k]) ? (rules[k] as any[]) : []);
  for (const r of arr("teacher_class_blocklist"))
    out.push(`Teacher ${r.sling_user_id} — never ${r.class_name}${r.reason ? ` (${r.reason})` : ""}`);
  for (const r of arr("teacher_slot_blocklist"))
    out.push(`Teacher ${r.sling_user_id} — never ${r.weekday} ${r.time}${r.reason ? ` (${r.reason})` : ""}`);
  for (const r of arr("priority_slots"))
    out.push(`Teacher ${r.sling_user_id} — preferred for ${r.weekday} ${r.time}`);
  for (const r of arr("slot_class_overrides"))
    out.push(`${r.weekday} ${r.time} is always ${r.class_name}`);
  const vpm = rules["variety_penalty_multiplier"] as Record<string, number> | undefined;
  for (const [uid, mult] of Object.entries(vpm ?? {}))
    out.push(`Teacher ${uid} — variety penalty ×${mult}`);
  if (rules["variety_penalty_per_class"] != null)
    out.push(`Variety penalty per class: ${rules["variety_penalty_per_class"]}`);
  for (const [from, to] of Object.entries((rules["sat_time_shifts"] as Record<string, string>) ?? {}))
    out.push(`Saturday ${from} moves to ${to}`);
  for (const [from, to] of Object.entries((rules["sun_time_shifts"] as Record<string, string>) ?? {}))
    out.push(`Sunday ${from} moves to ${to}`);
  return out;
}

function scriptBadge(v: AlgorithmVersion): string {
  if (!v.script_file) return "baseline script";
  if (v.script_missing) return "script deleted";
  if (v.script_archived) return "script archived";
  return v.script_file;
}

/** Active algorithm version + adoption history, with manual script deletion. */
export function AlgorithmCard({ refreshToken }: Props) {
  const [versions, setVersions] = useState<AlgorithmVersion[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);

  const refresh = () =>
    api.listAlgorithmVersions().then(setVersions).catch((e) => setError(String(e)));

  useEffect(() => {
    refresh();
  }, [refreshToken]);

  const active = versions?.[0] ?? null;
  const activeLabel = active ? `v${active.version}` : "v9";

  const onDelete = async (version: number) => {
    if (confirmDelete !== version) {
      setConfirmDelete(version);
      return;
    }
    setConfirmDelete(null);
    setError(null);
    try {
      await api.deleteAlgorithmScript(version);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="card">
      <strong>Algorithm</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        The rule set and script the next Generate will use. Versions are
        adopted from Claude's proposals (or your own) and never change once
        adopted.
      </p>
      <div style={{ marginTop: 10 }}>
        Active: <strong>{activeLabel}</strong>
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
      {active && (
        <div style={{ marginTop: 8 }}>
          <button className="disclosure" onClick={() => setExpanded(!expanded)}>
            {expanded ? "Hide rules" : "Show rules"}
          </button>
          {expanded && (
            <ul style={{ margin: "4px 0 0", paddingLeft: 20 }}>
              {ruleLines(active.rules).map((line, i) => (
                <li key={i} style={{ fontSize: 13 }}>{line}</li>
              ))}
              {ruleLines(active.rules).length === 0 && (
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
                <td>v{v.version}{v === active ? <span className="badge" style={{ marginLeft: 6 }}>active</span> : ""}</td>
                <td>{v.description}</td>
                <td className="muted">{v.created_by}</td>
                <td className="muted">{v.adopted_at.slice(0, 10)}</td>
                <td className="muted">{v.last_used_month ?? "—"}</td>
                <td className="muted"><code style={{ fontSize: 11 }}>{scriptBadge(v)}</code></td>
                <td>
                  {v.script_file && !v.script_missing && v !== active && (
                    <button className="btn-ghost btn-sm" onClick={() => onDelete(v.version)}>
                      {confirmDelete === v.version ? "Really delete?" : "Delete script"}
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {error && <div className="error">{error}</div>}
    </div>
  );
}
