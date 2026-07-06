import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Check, Upload } from "lucide-react";
import { api } from "../lib/api";
import { ProgressBar } from "./ui/ProgressBar";
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

  // Subscribe to progress before executing; clean up on unmount. If the
  // modal unmounts before listen() resolves, unlisten immediately instead
  // of leaking the subscription.
  useEffect(() => {
    let unmounted = false;
    listen<PushProgress>("push-progress", (e) => setProgress(e.payload)).then((u) => {
      if (unmounted) u();
      else unlisten.current = u;
    });
    return () => {
      unmounted = true;
      unlisten.current?.();
    };
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
        {phase === "loading" && (
          <>
            <h3>Push to Sling</h3>
            <p className="muted">Checking what's already in Sling…</p>
          </>
        )}

        {phase === "preview" && preview && (
          <>
            <h3>Push to Sling</h3>
            <p className="muted" style={{ marginTop: 0 }}>
              This creates{" "}
              <strong style={{ color: "var(--text-body)" }}>
                {preview.to_create.length} unpublished shift{preview.to_create.length === 1 ? "" : "s"}
              </strong>{" "}
              in Sling for {monthLabel}. Nothing goes live until you publish them in Sling.
              {preview.skipped_count > 0 && (
                <> <span className="muted">{preview.skipped_count} already in Sling (skipped).</span></>
              )}
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
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 18 }}>
              <button className="btn-ghost" onClick={onClose}>Cancel</button>
              <button className="btn-primary" onClick={onConfirm} disabled={preview.to_create.length === 0}>
                <Upload size={15} /> Push {preview.to_create.length} shift{preview.to_create.length === 1 ? "" : "s"}
              </button>
            </div>
          </>
        )}

        {phase === "pushing" && (
          <>
            <h3>Pushing to Sling…</h3>
            <p className="muted">
              Creating planning shifts for {monthLabel} in batches of 10 with pauses
              (Sling rate-limits). Don't close this window.
            </p>
            <ProgressBar value={pct} />
            {progress && (
              <p className="muted" style={{ fontVariantNumeric: "tabular-nums" }}>
                {progress.done}/{progress.total} — {progress.created} created
                {progress.failed > 0 && <>, {progress.failed} failed</>}
                {progress.last_label && <><br /><code>{progress.last_outcome}: {progress.last_label}</code></>}
              </p>
            )}
          </>
        )}

        {phase === "done" && summary && (
          <>
            <span className="bk-done-icon">
              <Check size={24} />
            </span>
            <h3 style={{ marginTop: 12 }}>Pushed to Sling</h3>
            <p className="muted" style={{ marginTop: 0 }}>
              {summary.created} planning shift{summary.created === 1 ? "" : "s"} created for {monthLabel}
              {summary.skipped > 0 && <>, {summary.skipped} already present</>}
              {summary.failed > 0 && <>, {summary.failed} failed</>}. Open Sling to review and publish them.
            </p>
            {summary.failed > 0 && (
              <p className="muted">
                Some shifts failed — click Push again to retry; the ones already created are skipped automatically.
              </p>
            )}
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 18 }}>
              <button className="btn-primary" onClick={onClose}>Done</button>
            </div>
          </>
        )}

        {phase === "error" && (
          <>
            <h3>Push to Sling</h3>
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
