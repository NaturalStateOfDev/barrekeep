// Top-of-app banner shown on launch when a newer signed release is available.
// Silent if none is available, in dev, or if the check fails. Shares the
// install flow with the Settings "Updates" card via lib/updater.

import { useEffect, useState } from "react";
import {
  checkForUpdate,
  installUpdate,
  type Update,
  type DownloadProgress,
} from "../lib/updater";

export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    // Silent startup check — never interrupt launch on failure.
    checkForUpdate()
      .then((u) => {
        if (!cancelled) setUpdate(u);
      })
      .catch(() => {
        /* offline / not configured / dev — stay quiet */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!update || dismissed) return null;

  const onInstall = async () => {
    setInstalling(true);
    setError(null);
    try {
      await installUpdate(update, setProgress);
      // installUpdate relaunches on success; reaching here is unusual.
    } catch (e) {
      setError(String(e));
      setInstalling(false);
    }
  };

  const pct = progress?.percent;

  return (
    <div
      role="status"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "8px 16px",
        background: "hsl(210 60% 96%)",
        borderBottom: "1px solid hsl(210 40% 80%)",
        fontSize: 14,
      }}
    >
      <span style={{ flex: 1 }}>
        {installing ? (
          <>
            Installing <strong>v{update.version}</strong>
            {pct != null ? ` — ${pct}%` : "…"} The app will restart.
          </>
        ) : error ? (
          <span style={{ color: "hsl(0 65% 38%)" }}>Update failed: {error}</span>
        ) : (
          <>
            <strong>Barrekeep v{update.version}</strong> is available.
          </>
        )}
      </span>

      {installing && pct != null && (
        <div
          aria-hidden
          style={{
            width: 120,
            height: 6,
            borderRadius: 3,
            background: "hsl(210 30% 85%)",
            overflow: "hidden",
          }}
        >
          <div
            style={{
              width: `${pct}%`,
              height: "100%",
              background: "hsl(210 70% 50%)",
              transition: "width 0.2s",
            }}
          />
        </div>
      )}

      {!installing && (
        <>
          <button className="btn-primary" onClick={onInstall}>
            Install &amp; restart
          </button>
          <button className="btn-ghost" onClick={() => setDismissed(true)}>
            Later
          </button>
        </>
      )}
    </div>
  );
}
