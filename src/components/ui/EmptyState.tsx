import type { LucideIcon } from "lucide-react";

interface Props {
  icon: LucideIcon;
  title: string;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}

export function EmptyState({ icon: Icon, title, message, actionLabel, onAction }: Props) {
  return (
    <div className="bk-empty">
      <span className="bk-empty-icon">
        <Icon size={24} />
      </span>
      <div className="bk-empty-title">{title}</div>
      <div className="bk-empty-message">{message}</div>
      {actionLabel && (
        <div style={{ marginTop: 10 }}>
          <button className="btn-primary" onClick={onAction}>{actionLabel}</button>
        </div>
      )}
    </div>
  );
}
