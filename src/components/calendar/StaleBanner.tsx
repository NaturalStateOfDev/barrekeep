import { AlertTriangle } from "lucide-react";

interface Props {
  lastPulledAt: string;
  generatedAt: string;
  onRegenerate: () => void;
}

export function StaleBanner({ lastPulledAt, onRegenerate }: Props) {
  return (
    <div className="bk-stale-banner">
      <AlertTriangle size={16} style={{ color: "var(--color-warning)", flexShrink: 0 }} />
      <span>This proposal was generated before the latest Sling pull ({prettyAgo(lastPulledAt)}).</span>
      <button className="btn-ghost btn-sm" onClick={onRegenerate}>Regenerate</button>
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
