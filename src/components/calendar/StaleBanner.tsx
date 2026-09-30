import { AlertTriangle, RefreshCw } from "lucide-react";

interface Props {
  lastPulledAt: string;
  generatedAt: string;
  refreshing?: boolean;
  readonly?: boolean;
  /** Re-pull availability/leave and re-check this draft — keeps edits. */
  onRefreshAvailability: () => void;
  /** Throw the draft away and generate again. */
  onRegenerate: () => void;
}

export function StaleBanner({ lastPulledAt, refreshing, readonly, onRefreshAvailability, onRegenerate }: Props) {
  return (
    <div className="bk-stale-banner">
      <AlertTriangle size={16} style={{ color: "var(--color-warning)", flexShrink: 0 }} />
      <span>
        Sling data changed since this draft was generated or last checked ({prettyAgo(lastPulledAt)}).
        Refresh availability to see what it affects — your edits are kept.
      </span>
      <button className="btn-primary btn-sm" onClick={onRefreshAvailability} disabled={refreshing || readonly}>
        <RefreshCw size={14} /> {refreshing ? "Refreshing…" : "Refresh availability"}
      </button>
      <button
        className="btn-ghost btn-sm"
        onClick={onRegenerate}
        disabled={refreshing || readonly}
        title="Generate a new draft from scratch (edits to this one are not carried over)"
      >
        Regenerate
      </button>
    </div>
  );
}

function prettyAgo(iso: string): string {
  const then = new Date(iso).getTime();
  const ms = Date.now() - then;
  const mins = Math.round(ms / 60000);
  if (mins < 60) return `${mins} min ago`;
  const hrs = Math.round(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.round(hrs / 24)}d ago`;
}
