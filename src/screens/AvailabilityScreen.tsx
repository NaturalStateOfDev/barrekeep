// Availability view: a teacher × day grid for one month. Each cell shows
// whether the teacher is available for the studio day, partly available,
// unavailable (incl. recurring sets and ones pending approval in Sling) or on
// leave; click a cell for the windows and blocks behind it.
//
// Data: computed windows + day spans from `get_month_availability`
// (teacher_availability_windows), blocks from `list_availability_blocks`.

import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, CalendarClock } from "lucide-react";
import { api } from "../lib/api";
import { MonthSelector } from "../components/MonthSelector";
import { PageHead } from "../components/ui/PageHead";
import { EmptyState } from "../components/ui/EmptyState";
import { Avatar } from "../components/ui/Avatar";
import {
  buildAvailabilityGrid,
  CELL_STATUS_LABEL,
  describeCell,
  type CellStatus,
  type GridCell,
} from "../lib/availability";
import { monthLabel, prettyDayLong, WEEKDAYS_SHORT } from "../lib/dates";
import { useTimeFormat } from "../lib/timeFormat";
import type { AvailabilityBlock, MonthAvailability, Teacher } from "../types";

const LEGEND: CellStatus[] = ["available", "partial", "unavailable", "pending", "leave", "closed"];

function nextMonth(todayIso: string): string {
  const [y, m] = todayIso.split("-").map(Number);
  return m === 12 ? `${y + 1}-01` : `${y}-${String(m + 1).padStart(2, "0")}`;
}

export function AvailabilityScreen({ onGoSettings }: { onGoSettings: () => void }) {
  const today = new Date().toISOString().slice(0, 10);
  const tf = useTimeFormat();
  const [month, setMonth] = useState(() => nextMonth(today));
  const [teachers, setTeachers] = useState<Teacher[]>([]);
  const [availability, setAvailability] = useState<MonthAvailability | null>(null);
  const [blocks, setBlocks] = useState<AvailabilityBlock[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<{ userId: number; date: string } | null>(null);

  useEffect(() => {
    let stale = false;
    setError(null);
    setSelected(null);
    setAvailability(null);
    Promise.all([api.listTeachers(), api.getMonthAvailability(month), api.listAvailabilityBlocks(month)])
      .then(([t, a, b]) => {
        if (stale) return;
        setTeachers(t);
        setAvailability(a);
        setBlocks(b);
      })
      .catch((e) => {
        if (!stale) setError(String(e));
      });
    return () => {
      stale = true;
    };
  }, [month]);

  const grid = useMemo(
    () => (availability ? buildAvailabilityGrid(month, teachers, availability, blocks) : null),
    [month, teachers, availability, blocks],
  );

  const selectedRow = selected && grid ? grid.rows.find((r) => r.teacher.sling_user_id === selected.userId) : undefined;
  const selectedCell: GridCell | undefined = selectedRow?.cells.find((c) => c.date === selected?.date);

  return (
    <div>
      <PageHead
        title="Availability"
        sub={`When each teacher can teach in ${monthLabel(month)} — studio hours minus unavailability and leave from Sling.`}
        actions={<MonthSelector today={today} value={month} onChange={setMonth} />}
      />
      {error && <div className="card error">{error}</div>}

      {availability?.warnings.map((w) => (
        <div key={w} className="bk-warn" style={{ marginBottom: 12 }}>
          <AlertTriangle size={15} /> {w}
        </div>
      ))}
      {availability && availability.set_issues.length > 0 && (
        <div className="card" style={{ marginBottom: 12 }}>
          <strong>Availability sets Barrekeep couldn't interpret</strong>
          <p className="muted" style={{ marginTop: 4 }}>
            These recurring entries from Sling produce no unavailability here. Check them in Sling, or send
            the newest raw pull file (Settings → Backups → Open raw pulls folder) to whoever maintains the app.
          </p>
          <table style={{ marginTop: 8 }}>
            <thead>
              <tr>
                <th style={{ textAlign: "left" }}>Teacher</th>
                <th style={{ textAlign: "left" }}>Set</th>
                <th style={{ textAlign: "left" }}>Repeats</th>
                <th style={{ textAlign: "left" }}>Problem</th>
              </tr>
            </thead>
            <tbody>
              {availability.set_issues.map((s, i) => (
                <tr key={i}>
                  <td>{s.teacher_name ?? `user ${s.sling_user_id}`}</td>
                  <td>{s.name ?? <span className="muted">unnamed</span>}</td>
                  <td><code>{s.interval_raw ?? "—"}</code></td>
                  <td className="muted">{s.problem}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {grid && grid.rows.length === 0 ? (
        <div className="card">
          <EmptyState
            icon={CalendarClock}
            title="No roster yet"
            message="Availability is computed for the teachers pulled from Sling. Connect Sling in Settings and pull a month first."
            actionLabel="Open Settings"
            onAction={onGoSettings}
          />
        </div>
      ) : grid && availability ? (
        <>
          <div className="bk-avail-meta">
            <div className="bk-avail-legend" aria-label="legend">
              {LEGEND.map((s) => (
                <span key={s} className="bk-avail-legend-item">
                  <span className={`bk-avail-swatch bk-avail-${s}`} /> {CELL_STATUS_LABEL[s]}
                </span>
              ))}
            </div>
            <div className="muted" style={{ fontSize: 12 }}>
              {availability.hours_set ? (
                "Studio hours from Settings, widened on days a class runs outside them."
              ) : (
                <>
                  Studio hours aren't set — each day spans its earliest to latest class.{" "}
                  <button className="btn-link" onClick={onGoSettings}>Set studio hours</button>
                </>
              )}
              {availability.set_count > 0 &&
                ` ${availability.set_count} recurring availability set${availability.set_count === 1 ? "" : "s"} from Sling` +
                  (availability.pending_set_count > 0 ? ` (${availability.pending_set_count} pending approval).` : ".")}
            </div>
          </div>

          <div className="card bk-avail-card">
            <div
              className="bk-avail-grid"
              style={{ gridTemplateColumns: `minmax(150px, 190px) repeat(${grid.dates.length}, minmax(22px, 1fr))` }}
              role="grid"
              aria-label={`Teacher availability for ${monthLabel(month)}`}
            >
              <div className="bk-avail-corner" />
              {grid.dates.map((date) => {
                const d = new Date(`${date}T12:00:00Z`);
                const dow = d.getUTCDay();
                return (
                  <div key={date} className={`bk-avail-head${dow === 0 || dow === 6 ? " bk-weekend" : ""}`}>
                    <span>{WEEKDAYS_SHORT[dow].slice(0, 2)}</span>
                    <span className="bk-avail-daynum">{d.getUTCDate()}</span>
                  </div>
                );
              })}
              {grid.rows.map((row) => (
                <div key={row.teacher.sling_user_id} style={{ display: "contents" }} role="row">
                  <div className="bk-avail-teacher">
                    <Avatar name={row.teacher.display_name} size={22} />
                    <span className="bk-avail-name">{row.teacher.display_name}</span>
                    {row.limitedDays > 0 && (
                      <span className="bk-avail-count" title={`${row.limitedDays} day(s) with limits`}>
                        {row.limitedDays}
                      </span>
                    )}
                  </div>
                  {row.cells.map((cell) => {
                    const isSel = selected?.userId === row.teacher.sling_user_id && selected.date === cell.date;
                    return (
                      <button
                        key={cell.date}
                        role="gridcell"
                        className={`bk-avail-cell bk-avail-${cell.status}${isSel ? " bk-selected" : ""}`}
                        title={`${row.teacher.display_name} · ${prettyDayLong(cell.date)}\n${describeCell(cell, tf.fmt).join("\n")}`}
                        aria-label={`${row.teacher.display_name}, ${prettyDayLong(cell.date)}: ${CELL_STATUS_LABEL[cell.status]}`}
                        onClick={() =>
                          setSelected(isSel ? null : { userId: row.teacher.sling_user_id, date: cell.date })
                        }
                      />
                    );
                  })}
                </div>
              ))}
            </div>
          </div>

          {selectedRow && selectedCell && (
            <div className="card bk-avail-detail">
              <div className="row" style={{ justifyContent: "space-between" }}>
                <strong>
                  {selectedRow.teacher.display_name} · {prettyDayLong(selectedCell.date)}
                </strong>
                <span className={`bk-avail-pill bk-avail-${selectedCell.status}`}>
                  {CELL_STATUS_LABEL[selectedCell.status]}
                </span>
              </div>
              {selectedCell.range && (
                <div className="muted" style={{ marginTop: 4, fontSize: 12 }}>
                  Studio day {tf.range(selectedCell.range.open, selectedCell.range.close)}
                  {selectedCell.range.widened && " (widened for a class outside normal hours)"}
                </div>
              )}
              <ul className="bk-avail-lines">
                {describeCell(selectedCell, tf.fmt).map((line, i) => (
                  <li key={i}>{line}</li>
                ))}
              </ul>
            </div>
          )}
        </>
      ) : !error ? (
        <div className="card muted">Loading availability…</div>
      ) : null}
    </div>
  );
}
