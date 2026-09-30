import { useState } from "react";
import { Archive, ArchiveRestore, ChevronDown, CloudOff, Copy, GitCompare, Pencil, Upload } from "lucide-react";
import type { ProposalSummary } from "../../types";
import { useTimeFormat } from "../../lib/timeFormat";

interface Props {
  /** Drafts of the active month to list (archived ones only when shown), newest first. */
  drafts: ProposalSummary[];
  /** How many of the month's drafts are archived (for the toggle label). */
  archivedCount: number;
  showArchived: boolean;
  onToggleArchived: () => void;
  value: number;
  onChange: (id: number) => void;
  readonly: boolean;
  onDuplicate: () => void;
  onRename: () => void;
  onArchiveToggle: () => void;
  onUseForPush: () => void;
  /** Delete this (non-push) draft's planning shifts from Sling. */
  onRemoveFromSling: () => void;
  onCompare: () => void;
}

/** Pill next to the month title: switches between the month's drafts and
 *  holds the per-draft actions (duplicate, rename, archive, use for push). */
export function DraftSwitcher({
  drafts,
  archivedCount,
  showArchived,
  onToggleArchived,
  value,
  onChange,
  readonly,
  onDuplicate,
  onRename,
  onArchiveToggle,
  onUseForPush,
  onRemoveFromSling,
  onCompare,
}: Props) {
  const [open, setOpen] = useState(false);
  const tf = useTimeFormat();
  const current = drafts.find((v) => v.id === value);
  if (!current) return null;

  const act = (fn: () => void) => () => {
    setOpen(false);
    fn();
  };

  return (
    <div className="bk-switcher bk-version-switcher">
      <button
        className="bk-switcher-button version"
        onClick={() => setOpen((o) => !o)}
        title={`Draft #${current.id} · ${current.algorithm_version}`}
      >
        {current.name}
        {current.is_push_candidate && <span className="bk-push-badge">Push draft</span>}
        {current.archived && <span className="bk-archived-badge">Archived</span>}
        <ChevronDown size={14} />
      </button>
      {open && (
        <>
          <div className="bk-switcher-backdrop" onClick={() => setOpen(false)} />
          <div className="bk-switcher-menu">
            {drafts.map((v) => (
              <button
                key={v.id}
                className={`bk-switcher-item ${v.id === value ? "active" : ""}`}
                onClick={() => {
                  onChange(v.id);
                  setOpen(false);
                }}
              >
                <span>
                  {v.name}
                  <span className="meta">
                    {" "}· {v.algorithm_version} · {tf.timestamp(v.generated_at)}
                    {v.created_from === "duplicate" && " · copy"}
                  </span>
                </span>
                <span className="status">
                  {v.is_push_candidate ? (
                    <span className="bk-push-badge">Push draft</span>
                  ) : v.archived ? (
                    "archived"
                  ) : v.sling_shift_count > 0 ? (
                    `${v.sling_shift_count} in Sling`
                  ) : v.pushed ? (
                    "pushed before"
                  ) : (
                    ""
                  )}
                </span>
              </button>
            ))}
            {archivedCount > 0 && (
              <button className="bk-switcher-toggle" onClick={onToggleArchived}>
                {showArchived ? "Hide archived drafts" : `Show archived drafts (${archivedCount})`}
              </button>
            )}
            <div className="bk-switcher-divider" />
            <div className="bk-draft-actions">
              <button className="bk-switcher-new" onClick={act(onDuplicate)} disabled={readonly}>
                <Copy size={15} /> Duplicate “{current.name}”
              </button>
              <button className="bk-switcher-new" onClick={act(onRename)}>
                <Pencil size={15} /> Rename
              </button>
              <button
                className="bk-switcher-new"
                onClick={act(onUseForPush)}
                disabled={readonly || current.is_push_candidate || current.archived}
                title={
                  current.is_push_candidate
                    ? "Already the push draft"
                    : current.archived
                      ? "Unarchive it first"
                      : "Push to Sling will send this draft"
                }
              >
                <Upload size={15} /> Use for push
              </button>
              <button
                className="bk-switcher-new"
                onClick={act(onArchiveToggle)}
                disabled={current.is_push_candidate && !current.archived}
                title={
                  current.is_push_candidate
                    ? "The push draft can't be archived — pick another push draft first"
                    : ""
                }
              >
                {current.archived ? (
                  <>
                    <ArchiveRestore size={15} /> Unarchive
                  </>
                ) : (
                  <>
                    <Archive size={15} /> Archive
                  </>
                )}
              </button>
              {!current.is_push_candidate && current.sling_shift_count > 0 && (
                <button
                  className="bk-switcher-new"
                  onClick={act(onRemoveFromSling)}
                  disabled={readonly}
                  title="Delete this draft's planning shifts from Sling (published or Sling-edited shifts are left alone)"
                >
                  <CloudOff size={15} /> Remove its {current.sling_shift_count} shift
                  {current.sling_shift_count === 1 ? "" : "s"} from Sling…
                </button>
              )}
              <button className="bk-switcher-new" onClick={act(onCompare)} disabled={drafts.length < 2}>
                <GitCompare size={15} /> Compare drafts
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
