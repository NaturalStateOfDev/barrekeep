import { useState, type ReactNode } from "react";
import { MAX_DRAFT_NAME } from "../lib/drafts";

interface Props {
  title: string;
  initial: string;
  /** Shown when the field is empty; empty is allowed only when `optional`. */
  placeholder?: string;
  optional?: boolean;
  confirmLabel: string;
  hint?: ReactNode;
  /** Resolves when done; a rejection is shown in the modal. */
  onConfirm: (name: string) => Promise<void>;
  onCancel: () => void;
}

/** Small name prompt for new / duplicated / renamed drafts. */
export function DraftNameModal({
  title,
  initial,
  placeholder,
  optional = false,
  confirmLabel,
  hint,
  onConfirm,
  onCancel,
}: Props) {
  const [name, setName] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trimmed = name.trim();
  const canSubmit = !busy && (optional || trimmed.length > 0) && trimmed.length <= MAX_DRAFT_NAME;

  const submit = async () => {
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      await onConfirm(trimmed);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        <label className="field">
          <span>Draft name{optional ? " (optional)" : ""}</span>
          <input
            autoFocus
            value={name}
            maxLength={MAX_DRAFT_NAME}
            placeholder={placeholder}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
              if (e.key === "Escape" && !busy) onCancel();
            }}
            disabled={busy}
          />
        </label>
        {hint && <div className="muted" style={{ fontSize: 13 }}>{hint}</div>}
        {error && <div className="error">{error}</div>}
        <div className="row" style={{ justifyContent: "flex-end", marginTop: 18 }}>
          <button className="btn-ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button className="btn-primary" onClick={submit} disabled={!canSubmit}>
            {busy ? "Working…" : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
