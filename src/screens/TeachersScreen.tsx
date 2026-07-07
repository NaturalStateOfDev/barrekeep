import { useEffect, useState } from "react";
import { RefreshCw, Users } from "lucide-react";
import { api } from "../lib/api";
import { PageHead } from "../components/ui/PageHead";
import { EmptyState } from "../components/ui/EmptyState";
import { Avatar } from "../components/ui/Avatar";
import type { Teacher } from "../types";

export function TeachersScreen({ onGoSettings }: { onGoSettings: () => void }) {
  const [teachers, setTeachers] = useState<Teacher[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [syncMsg, setSyncMsg] = useState<string | null>(null);

  const refresh = () =>
    api.listTeachers().then(setTeachers).catch((e) => setError(String(e)));

  useEffect(() => { refresh(); }, []);

  const onRefreshRoster = async () => {
    setSyncing(true); setSyncMsg(null); setError(null);
    try {
      const s = await api.refreshRosterFromSling();
      setSyncMsg(`Synced from Sling: ${s.teachers_active} teachers, ${s.positions_active} class types` +
        (s.teachers_deactivated ? `, ${s.teachers_deactivated} deactivated` : "") + ".");
      await refresh();
    } catch (e) {
      const msg = String(e);
      if (msg.includes("sling-401")) setSyncMsg("Sling token expired — log in again (Settings), then Refresh.");
      else if (msg.includes("not configured")) setSyncMsg("Set Studio configuration in Settings first, then Refresh.");
      else setSyncMsg(`Refresh failed: ${msg}`);
    } finally { setSyncing(false); }
  };

  const activeTeachers = teachers ? teachers.filter((t) => t.active) : null;

  return (
    <div>
      <PageHead
        title="Teachers"
        sub="Roster pulled from Sling · qualifications drive scheduling"
        actions={
          <button className="btn-primary" onClick={onRefreshRoster} disabled={syncing}>
            <RefreshCw size={15} /> {syncing ? "Refreshing…" : "Refresh from Sling"}
          </button>
        }
      />
      {syncMsg && <div className="muted" style={{ marginBottom: 12 }}>{syncMsg}</div>}
      {error && <div className="card error">{error}</div>}
      {activeTeachers !== null && activeTeachers.length === 0 ? (
        <div className="card">
          <EmptyState
            icon={Users}
            title="No roster yet"
            message="Teachers, qualifications and weekly caps come from Sling. Connect Sling in Settings, then refresh to load your roster."
            actionLabel="Open Settings"
            onAction={onGoSettings}
          />
        </div>
      ) : activeTeachers && activeTeachers.length > 0 && (
        <div className="card" style={{ padding: "6px 4px" }}>
          <table>
            <thead>
              <tr>
                <th>Teacher</th>
                <th>Sling ID</th>
                <th>Location</th>
                <th>Target/wk</th>
                <th>Max/wk</th>
                <th>Lead</th>
              </tr>
            </thead>
            <tbody>
              {activeTeachers.map((t) => (
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
    </div>
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

  const keyHandler = (reset: () => void) => (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") (e.target as HTMLInputElement).blur();
    if (e.key === "Escape") {
      reset();
      (e.target as HTMLInputElement).blur();
    }
  };

  return (
    <tr>
      <td>
        <span style={{ display: "inline-flex", alignItems: "center", gap: 9 }}>
          <Avatar name={teacher.display_name} size={26} />
          {teacher.display_name}
        </span>
      </td>
      <td className="muted"><code>{teacher.sling_user_id}</code></td>
      <td>{teacher.locations || <span className="muted">—</span>}</td>
      <td>
        <input
          type="number"
          min={0}
          className="cell-input"
          value={target}
          disabled={saving}
          onChange={(e) => setTarget(e.target.value)}
          onBlur={commit}
          onKeyDown={keyHandler(() => setTarget(String(teacher.weekly_target)))}
        />
      </td>
      <td>
        <input
          type="number"
          min={0}
          className="cell-input"
          value={max}
          disabled={saving}
          onChange={(e) => setMax(e.target.value)}
          onBlur={commit}
          onKeyDown={keyHandler(() => setMax(String(teacher.weekly_max)))}
        />
      </td>
      <td>{teacher.is_lead ? <span className="badge">Lead</span> : ""}</td>
    </tr>
  );
}
