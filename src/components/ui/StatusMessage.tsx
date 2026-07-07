import type { ReactNode } from "react";

interface Props {
  tone?: "ok" | "error" | "muted";
  children: ReactNode;
}

/** Inline feedback line under an action (.ok / .error / .muted). */
export function StatusMessage({ tone = "muted", children }: Props) {
  return <div className={tone}>{children}</div>;
}
