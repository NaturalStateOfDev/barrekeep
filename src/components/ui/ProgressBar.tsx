interface Props {
  value: number;
  max?: number;
}

export function ProgressBar({ value, max = 100 }: Props) {
  const pct = max > 0 ? Math.max(0, Math.min(100, (value / max) * 100)) : 0;
  return (
    <div className="bk-progress">
      <div className="bk-progress-fill" style={{ width: `${pct}%` }} />
    </div>
  );
}
