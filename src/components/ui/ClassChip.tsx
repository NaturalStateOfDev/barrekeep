import { chipFor } from "../../lib/formatChips";

interface Props {
  className: string;
  size?: "sm" | "md";
}

/** The barre class-format color tag. The color is the format's identity —
 *  the mapping is fixed (see tokens/chips.css). */
export function ClassChip({ className, size = "sm" }: Props) {
  const chip = chipFor(className);
  return (
    <span
      className="bk-slot-chip"
      title={className}
      style={{
        background: `var(${chip.token})`,
        color: `var(${chip.token}-fg)`,
        ...(size === "md" ? { padding: "3px 10px", fontSize: "var(--text-sm)" } : null),
      }}
    >
      {chip.label}
    </span>
  );
}
