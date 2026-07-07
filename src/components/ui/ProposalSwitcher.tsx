import { useState } from "react";
import { ChevronDown, Plus } from "lucide-react";
import { monthLabel } from "../../lib/dates";

export interface MonthEntry {
  month: string; // "YYYY-MM"
  /** The month's current (or newest) proposal — selected when the month is picked. */
  proposalId: number;
  draftCount: number;
}

interface Props {
  months: MonthEntry[];
  /** Selected month, or null when starting a new month. */
  value: string | null;
  /** Title to show when no month is selected (new-month mode). */
  fallbackTitle: string;
  onChange: (proposalId: number) => void;
  onNew: () => void;
}

/** The Proposals page title: a serif month heading that drops down over the
 *  scheduled months, with a "Start a new month…" entry at the bottom.
 *  Draft/version history for the selected month lives in the sibling
 *  VersionSwitcher. */
export function ProposalSwitcher({ months, value, fallbackTitle, onChange, onNew }: Props) {
  const [open, setOpen] = useState(false);
  const current = value != null ? months.find((m) => m.month === value) : undefined;
  const title = current ? monthLabel(current.month) : fallbackTitle;

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
            {months.map((m) => (
              <button
                key={m.month}
                className={`bk-switcher-item ${m.month === value ? "active" : ""}`}
                onClick={() => {
                  onChange(m.proposalId);
                  setOpen(false);
                }}
              >
                <span>
                  {monthLabel(m.month)}
                  {m.draftCount > 1 && (
                    <span className="meta"> · {m.draftCount} drafts</span>
                  )}
                </span>
              </button>
            ))}
            {months.length > 0 && <div className="bk-switcher-divider" />}
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
