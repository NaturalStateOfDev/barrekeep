import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { PageHead } from "../components/ui/PageHead";
import { ClassChip } from "../components/ui/ClassChip";
import type { Position } from "../types";

export function PositionsScreen() {
  const [positions, setPositions] = useState<Position[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => api.listPositions().then(setPositions).catch((e) => setError(String(e)));

  useEffect(() => {
    refresh();
  }, []);

  return (
    <div>
      <PageHead
        title="Class types"
        sub="Uncheck non-class positions (e.g. Sales Rep) so they're excluded from the roster and scheduling."
      />
      {error && <div className="card error">{error}</div>}
      {positions && (
        <div className="card" style={{ padding: "6px 4px" }}>
          <table>
            <thead>
              <tr>
                <th>Class</th>
                <th>Sling position ID</th>
                <th>Duration</th>
                <th>Special</th>
                <th>Schedulable</th>
              </tr>
            </thead>
            <tbody>
              {positions.map((p) => (
                <tr key={p.sling_position_id}>
                  <td><ClassChip className={p.class_name} size="md" /></td>
                  <td className="muted">
                    <code>{p.sling_position_id}</code>
                  </td>
                  <td>{p.duration_minutes ? `${p.duration_minutes} min` : "—"}</td>
                  <td>{p.is_special ? "yes" : ""}</td>
                  <td>
                    <input
                      type="checkbox"
                      checked={p.active}
                      style={{ accentColor: "var(--accent)", width: 16, height: 16 }}
                      onChange={async (e) => {
                        setError(null);
                        try { await api.setPositionActive(p.sling_position_id, e.target.checked); await refresh(); }
                        catch (err) { setError(String(err)); }
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
