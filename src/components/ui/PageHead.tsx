import type { ReactNode } from "react";

interface Props {
  /** Serif page title, or custom node (e.g. the proposal switcher). */
  title: ReactNode;
  sub?: ReactNode;
  actions?: ReactNode;
}

export function PageHead({ title, sub, actions }: Props) {
  return (
    <div className="bk-page-head">
      <div>
        {typeof title === "string" ? <h1>{title}</h1> : title}
        {sub && <div className="bk-page-sub">{sub}</div>}
      </div>
      {actions && <div className="bk-page-actions">{actions}</div>}
    </div>
  );
}
