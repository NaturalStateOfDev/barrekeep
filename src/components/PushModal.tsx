import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type { PushPreview, PushProgress, PushSummary } from "../types";

interface Props {
  proposalId: number;
  monthLabel: string;
  onClose: () => void;
  onTokenExpired: () => void;
}

type Phase = "loading" | "preview" | "pushing" | "done" | "error";

export function PushModal({ proposalId, monthLabel, onClose, onTokenExpired }: Props) {
  const [phase, setPhase] = useState<Phase>("loading");
  const [preview, setPreview] = useState<PushPreview | null>(null);
  const [progress, setProgress] = useState<PushProgress | null>(null);
  const [summary, setSummary] = useState<PushSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const unlisten = useRef<(() => void) | null>(null);

  // Dry-run on mount.
  useEffect(() => {
    let cancelled = false;
    api.pushProposalDryRun(proposalId)
      .then((p) => { if (!cancelled) { setPreview(p); setPhase("preview"); } })
      .catch((e) => {
        if (!cancelled) {
          if (String(e).includes("sling-401")) onTokenExpired();
          else { setError(String(e)); setPhase("error"); }
        }
      });
    return () => { cancelled = true; };
  }, [proposalId]);

  // Subscribe to progress before executing; clean up on unmount.
  useEffect(() => {
    listen<PushProgress>("push-progress", (e) => setProgress(e.payload)).then((u) => { unlisten.current = u; });
    return () => { unlisten.current?.(); };
  }, []);

  const onConfirm = async () => {
    setPhase("pushing");
    setError(null);
    try {
      const s = await api.pushProposalExecute(proposalId);
      setSummary(s);
      setPhase("done");
    } catch (e) {
      if (String(e).includes("sling-401")) onTokenExpired();
      else { setError(String(e)); setPhase("error"); }
    }
  };

  const pct = progress && progress.total > 0
    ? Math.round((progress.done / progress.total) * 100) : 0;

  return (
    <div className="modal-backdrop" onClick={phase === "pushing" ? undefined : onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>Push {monthLabel} to Sling</h3>

        {phase === "loading" && <p className="muted">Checking what's already in Sling…</p>}

        {phase === "preview" && preview && (
          <>
            <p>
              <strong>{preview.to_create.length}</strong> shift(s) will be created as planning shifts.
              {preview.skipped_count > 0 && <> <span className="muted">{preview.skipped_count} already in Sling (skipped).</span></>}
            </p>
            {preview.to_create.length === 0 ? (
              <p className="ok">Everything is already in Sling — nothing to push.</p>
            ) : (
              <div className="bk-push-list">
                {preview.to_create.map((it, i) => (
                  <div className="bk-push-row" key={i}>
                    <span>{it.date} {it.start}–{it.end}</span>
                    <span>{it.class_name}</span>
                    <span>→ {it.teacher_name}</span>
                  </div>
                ))}
              </div>
            )}
            <div className="row" style={{ justifyContent: "space-between", marginTop: 12 }}>
              <button className="btn-ghost" onClick={onClose}>Cancel</button>
              <button className="btn-primary" onClick={onConfirm} disabled={preview.to_create.length === 0}>
                Confirm push ({preview.to_create.length})
              </button>
            </div>
          </>
        )}

        {phase === "pushing" && (
          <>
            <p className="muted">Pushing in batches of 10 with pauses (Sling rate-limits). Keep this window open.</p>
            <div className="bk-progress"><div className="bk-progress-fill" style={{ width: `${pct}%` }} /></div>
            {progress && (
              <p className="muted">
                {progress.done}/{progress.total} — {progress.created} created
                {progress.failed > 0 && <>, {progress.failed} failed</>}
                {progress.last_label && <><br /><code>{progress.last_outcome}: {progress.last_label}</code></>}
              </p>
            )}
          </>
        )}

        {phase === "done" && summary && (
          <>
            <p className="ok"><strong>Done.</strong> {summary.created} created, {summary.failed} failed, {summary.skipped} already present.</p>
            {summary.failed > 0 && <p className="muted">Some shifts failed — click Push again to retry; the ones already created are skipped automatically.</p>}
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 12 }}>
              <button className="btn-primary" onClick={onClose}>Close</button>
            </div>
          </>
        )}

        {phase === "error" && (
          <>
            <div className="error">{error}</div>
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 12 }}>
              <button className="btn-ghost" onClick={onClose}>Close</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
