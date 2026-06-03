interface Props {
  lastPulledAt: string;
  generatedAt: string;
  onRegenerate: () => void;
}

export function StaleBanner({ lastPulledAt, onRegenerate }: Props) {
  return (
    <div style={{
      padding: "0.5rem 0.75rem",
      background: "hsl(36 60% 92%)",
      border: "1px solid hsl(36 60% 70%)",
      borderRadius: "var(--radius)",
      marginBottom: "0.5rem",
      display: "flex",
      alignItems: "center",
      gap: "0.75rem",
    }}>
      <span>&#9888; This proposal was generated before the latest Sling pull ({prettyAgo(lastPulledAt)}).</span>
      <button className="btn-ghost" onClick={onRegenerate}>Regenerate</button>
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
