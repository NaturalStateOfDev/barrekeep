import { useState } from "react";
import { ChevronDown } from "lucide-react";
import type { ProposalSummary } from "../../types";
import { formatTimestamp } from "../../lib/dates";

interface Props {
  /** Proposals for the active month, newest first. */
  versions: ProposalSummary[];
  value: number;
  onChange: (id: number) => void;
}

/** Compact pill next to the month title that switches between the drafts
 *  generated for that month. */
export function VersionSwitcher({ versions, value, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const current = versions.find((v) => v.id === value);
  if (!current) return null;

  return (
    <div className="bk-switcher bk-version-switcher">
      <button className="bk-switcher-button version" onClick={() => setOpen((o) => !o)}>
        Draft #{current.id} · {current.algorithm_version}
        <ChevronDown size={14} />
      </button>
      {open && (
        <>
          <div className="bk-switcher-backdrop" onClick={() => setOpen(false)} />
          <div className="bk-switcher-menu">
            {versions.map((v) => (
              <button
                key={v.id}
                className={`bk-switcher-item ${v.id === value ? "active" : ""}`}
                onClick={() => {
                  onChange(v.id);
                  setOpen(false);
                }}
              >
                <span>
                  Draft #{v.id}
                  <span className="meta">
                    {" "}· {v.algorithm_version} · {formatTimestamp(v.generated_at)}
                  </span>
                </span>
                <span className="status">{v.is_current ? "current" : "superseded"}</span>
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
