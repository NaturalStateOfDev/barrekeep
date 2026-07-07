import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

interface Props {
  label: string;
  value: ReactNode;
  unit?: string;
  icon?: LucideIcon;
  /** Soft background of the icon box. */
  tint?: string;
  /** Ink for the value and icon (defaults to body text). */
  ink?: string;
  /** Replaces the icon box (e.g. the coverage ring). */
  ring?: ReactNode;
}

export function Kpi({ label, value, unit, icon: Icon, tint, ink, ring }: Props) {
  return (
    <div className="card bk-kpi">
      <div>
        <div className="bk-kpi-label">{label}</div>
        <div className="bk-kpi-value" style={ink ? { color: ink } : undefined}>
          {value}
          {unit && <small>{unit}</small>}
        </div>
      </div>
      {ring ?? (Icon && (
        <span className="bk-kpi-icon" style={{ background: tint }}>
          <Icon size={17} color={ink} />
        </span>
      ))}
    </div>
  );
}

/** Slim plum progress ring for the Coverage KPI. */
export function CoverageRing({ pct }: { pct: number }) {
  const C = 2 * Math.PI * 19; // r = 19
  const clamped = Math.max(0, Math.min(100, pct));
  return (
    <svg width="42" height="42" viewBox="0 0 46 46" role="img" aria-label={`${clamped}% coverage`}>
      <circle cx="23" cy="23" r="19" fill="none" stroke="var(--surface-sunken)" strokeWidth="5" />
      <circle
        cx="23"
        cy="23"
        r="19"
        fill="none"
        stroke="var(--accent)"
        strokeWidth="5"
        strokeLinecap="round"
        strokeDasharray={C}
        strokeDashoffset={C * (1 - clamped / 100)}
        transform="rotate(-90 23 23)"
      />
    </svg>
  );
}
