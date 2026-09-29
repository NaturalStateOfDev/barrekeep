// Top-of-app warning when this session's routine database backup failed
// (e.g. the startup backup). Non-fatal by design — the app keeps working —
// but the user should know their safety net is missing. Silent otherwise.

import { useEffect, useState } from "react";
import { api } from "../lib/api";

export function BackupWarningBanner({ onGoSettings }: { onGoSettings: () => void }) {
  const [warning, setWarning] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api.listBackups()
      .then((info) => { if (!cancelled) setWarning(info.last_error); })
      .catch(() => { /* never interrupt launch over this */ });
    return () => { cancelled = true; };
  }, []);

  if (!warning || dismissed) return null;

  return (
    <div
      role="status"
      className="bk-update-banner"
      style={{ background: "var(--color-warning-bg)", borderColor: "var(--color-warning-stroke)" }}
    >
      <span style={{ flex: 1, color: "var(--color-warning)" }}>
        Database backup failed: {warning}
      </span>
      <button className="btn-ghost btn-sm" onClick={() => { setDismissed(true); onGoSettings(); }}>
        Backups…
      </button>
      <button className="btn-ghost btn-sm" onClick={() => setDismissed(true)}>
        Dismiss
      </button>
    </div>
  );
}
