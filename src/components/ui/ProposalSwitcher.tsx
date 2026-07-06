import { useState } from "react";
import { ChevronDown, Plus } from "lucide-react";
import type { ProposalSummary } from "../../types";
import { monthLabel } from "../../lib/dates";

interface Props {
  proposals: ProposalSummary[];
  /** Selected proposal id, or null when starting a new month. */
  value: number | null;
  /** Title to show when no proposal is selected (new-month mode). */
  fallbackTitle: string;
  onChange: (id: number) => void;
  onNew: () => void;
}

/** The Proposals page title: a serif month heading that drops down over the
 *  proposal history, with a "Start a new month…" entry at the bottom. */
export function ProposalSwitcher({ proposals, value, fallbackTitle, onChange, onNew }: Props) {
  const [open, setOpen] = useState(false);
  const current = value != null ? proposals.find((p) => p.id === value) : undefined;
  const title = current ? monthLabel(current.target_month) : fallbackTitle;

  return (
    <div className="bk-switcher">
      <button className="bk-switcher-button" onClick={() => setOpen((o) => !o)}>
        {title}
        <ChevronDown size={22} />
      </button>
      {open && (
        <>
          <div className="bk-switcher-backdrop" onClick={() => setOpen(false)} />
          <div className="bk-switcher-menu">
            {proposals.map((p) => (
              <button
                key={p.id}
                className={`bk-switcher-item ${p.id === value ? "active" : ""}`}
                onClick={() => {
                  onChange(p.id);
                  setOpen(false);
                }}
              >
                <span>
                  {monthLabel(p.target_month)}{" "}
                  <span className="meta">· #{p.id} · {p.algorithm_version}</span>
                </span>
                <span className="status">{p.is_current ? "current" : "superseded"}</span>
              </button>
            ))}
            {proposals.length > 0 && <div className="bk-switcher-divider" />}
            <button
              className="bk-switcher-new"
              onClick={() => {
                onNew();
                setOpen(false);
              }}
            >
              <Plus size={16} /> Start a new month…
            </button>
          </div>
        </>
      )}
    </div>
  );
}
