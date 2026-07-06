import type { ReactNode } from "react";

interface Props {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
  style?: React.CSSProperties;
}

/** Labeled form control: small uppercase caption above a native control. */
export function Field({ label, hint, children, style }: Props) {
  return (
    <label className="field" style={style}>
      <span>{label}</span>
      {children}
      {hint && <span className="hint">{hint}</span>}
    </label>
  );
}
