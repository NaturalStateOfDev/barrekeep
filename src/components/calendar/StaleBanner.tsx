interface Props {
  lastPulledAt: string;
  generatedAt: string;
  onRegenerate: () => void;
}

export function StaleBanner({ lastPulledAt, onRegenerate }: Props) {
  return (
    <div className="bk-stale-banner">
      <span>&#9888; This proposal was generated before the latest Sling pull ({prettyAgo(lastPulledAt)}).</span>
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
