import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Check, Trash2, Upload } from "lucide-react";
import { api } from "../lib/api";
import { groupSyncActions, hasWork, shiftLabel, slingChangeCount, SYNC_KIND_LABEL, updateLabel } from "../lib/sync";
import { ProgressBar } from "./ui/ProgressBar";
import type { SyncAction, SyncPreview, SyncProgress, SyncSummary } from "../types";

interface Props {
  /** "push": sync the month's push draft. "remove": delete this draft's shifts from Sling. */
  mode: "push" | "remove";
  proposalId: number;
  draftName: string;
  monthLabel: string;
  onClose: () => void;
  onTokenExpired: () => void;
}

type Phase = "loading" | "preview" | "running" | "done" | "error";

function ActionRow({ a }: { a: SyncAction }) {
  const main = a.after ?? a.before;
  return (
    <div className={`bk-push-row bk-sync-${a.kind}`}>
      <span>{main ? shiftLabel(main) : `Sling shift ${a.sling_shift_id ?? ""}`}</span>
      <span className="muted">
        {a.kind === "update" ? updateLabel(a) : a.from_draft ? `from “${a.from_draft}”` : ""}
        {a.kind === "skip" && a.reason}
      </span>
    </div>
  );
}

function Section({ title, items, note }: { title: string; items: SyncAction[]; note?: string }) {
  if (items.length === 0) return null;
  return (
    <div className="bk-sync-section">
      <div className="bk-sync-section-head">
        {title} <span className="badge">{items.length}</span>
        {note && <span className="muted"> — {note}</span>}
      </div>
      <div className="bk-push-list">
        {items.map((a, i) => (
          <ActionRow a={a} key={`${a.kind}-${a.proposal_shift_id}-${a.sling_shift_id ?? ""}-${i}`} />
        ))}
      </div>
    </div>
  );
}

export function PushModal({ mode, proposalId, draftName, monthLabel, onClose, onTokenExpired }: Props) {
  const [phase, setPhase] = useState<Phase>("loading");
  const [preview, setPreview] = useState<SyncPreview | null>(null);
  const [cleanup, setCleanup] = useState(false);
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [summary, setSummary] = useState<SyncSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const unlisten = useRef<(() => void) | null>(null);

  const fail = (e: unknown) => {
    if (String(e).includes("sling-401")) onTokenExpired();
    else {
      setError(String(e));
      setPhase("error");
    }
  };

  // Plan on mount, and again whenever the cleanup option changes (it changes
  // what's deleted and therefore what counts as a duplicate).
  useEffect(() => {
    let cancelled = false;
    setPhase("loading");
    const req =
      mode === "push" ? api.pushSyncPreview(proposalId, cleanup) : api.removeDraftFromSlingPreview(proposalId);
    req
      .then((p) => {
        if (!cancelled) {
          setPreview(p);
          setPhase("preview");
        }
      })
      .catch((e) => {
        if (!cancelled) fail(e);
      });
    return () => {
      cancelled = true;
    };
  }, [proposalId, mode, cleanup]);

  // Subscribe to progress before executing; clean up on unmount.
  useEffect(() => {
    let unmounted = false;
    listen<SyncProgress>("push-progress", (e) => setProgress(e.payload)).then((u) => {
      if (unmounted) u();
      else unlisten.current = u;
    });
    return () => {
      unmounted = true;
      unlisten.current?.();
    };
  }, []);

  const onConfirm = async () => {
    if (!preview) return;
    setPhase("running");
    setError(null);
    try {
      const s =
        mode === "push"
          ? await api.pushSyncExecute(proposalId, cleanup, preview.plan_key)
          : await api.removeDraftFromSlingExecute(proposalId, preview.plan_key);
      setSummary(s);
      setPhase("done");
    } catch (e) {
      fail(e);
    }
  };

  const pct = progress && progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0;
  const title = mode === "push" ? `Push “${draftName}” to Sling` : `Remove “${draftName}” from Sling`;

  return (
    <div className="modal-backdrop" onClick={phase === "running" ? undefined : onClose}>
      <div className="modal bk-sync-modal" onClick={(e) => e.stopPropagation()}>
        {phase === "loading" && (
          <>
            <h3>{title}</h3>
            <p className="muted">Checking what's in Sling…</p>
          </>
        )}

        {phase === "preview" && preview && (() => {
          const g = groupSyncActions(preview.actions);
          const changes = slingChangeCount(preview);
          const work = hasWork(preview);
          return (
            <>
              <h3>{title}</h3>
              <p className="muted" style={{ marginTop: 0 }}>
                {mode === "push" ? (
                  <>
                    Only what changed is sent, as unpublished (planning) shifts for {monthLabel}. Nothing
                    goes live until you publish in Sling.
                    {preview.unchanged > 0 && <> {preview.unchanged} shift{preview.unchanged === 1 ? " is" : "s are"} already up to date.</>}
                  </>
                ) : (
                  <>
                    Deletes this draft's planning shifts for {monthLabel} from Sling. Shifts that were
                    published or edited in Sling are left alone.
                  </>
                )}
              </p>

              {mode === "push" && preview.cleanup_offers.length > 0 && (() => {
                const removable = preview.cleanup_offers.reduce((n, o) => n + o.removable, 0);
                const blocked = preview.cleanup_offers.reduce((n, o) => n + o.blocked, 0);
                const names = preview.cleanup_offers.map((o) => `“${o.draft_name}”`).join(", ");
                return (
                  <label className="bk-sync-cleanup">
                    <input
                      type="checkbox"
                      checked={cleanup}
                      disabled={removable === 0 && !cleanup}
                      onChange={(e) => setCleanup(e.target.checked)}
                    />
                    <span>
                      Remove {removable} planning shift{removable === 1 ? "" : "s"} previously pushed from {names}
                      {blocked > 0 && (
                        <span className="muted"> ({blocked} published or edited in Sling — left alone)</span>
                      )}
                      <br />
                      <span className="muted">
                        Removed before this draft's new shifts are created. Leave unchecked to keep them in Sling.
                      </span>
                    </span>
                  </label>
                );
              })()}

              {!work ? (
                <p className="ok">Sling already matches “{preview.draft_name}” — nothing to send.</p>
              ) : (
                <div className="bk-sync-sections">
                  <Section title={SYNC_KIND_LABEL.create} items={g.create} />
                  <Section title={SYNC_KIND_LABEL.update} items={g.update} note="replaced (delete + re-create)" />
                  <Section title={SYNC_KIND_LABEL.delete} items={g.delete} />
                  <Section title={SYNC_KIND_LABEL.cleanup} items={g.cleanup} />
                  <Section title={SYNC_KIND_LABEL.adopt} items={g.adopt} note="no change in Sling" />
                  <Section
                    title={SYNC_KIND_LABEL.baseline}
                    items={g.baseline}
                    note="matches the draft — syncable from now on, no change in Sling"
                  />
                </div>
              )}
              {g.deletedInSling.length > 0 && (
                <div className="bk-warn" style={{ marginTop: 12 }}>
                  <Section
                    title="Deleted in Sling since last push"
                    items={g.deletedInSling}
                    note="will be re-created on the next push unless you remove it from the draft"
                  />
                </div>
              )}
              {g.skip.length > 0 && (
                <div className="bk-warn" style={{ marginTop: 12 }}>
                  <Section title={SYNC_KIND_LABEL.skip} items={g.skip} note="the app won't touch these" />
                </div>
              )}

              <div className="row" style={{ justifyContent: "flex-end", marginTop: 18 }}>
                <button className="btn-ghost" onClick={onClose}>
                  Cancel
                </button>
                <button className="btn-primary" onClick={onConfirm} disabled={!work}>
                  {mode === "push" ? <Upload size={15} /> : <Trash2 size={15} />}{" "}
                  {changes > 0
                    ? `${mode === "push" ? "Send" : "Remove"} ${changes} change${changes === 1 ? "" : "s"}`
                    : "Record"}
                </button>
              </div>
            </>
          );
        })()}

        {phase === "running" && (
          <>
            <h3>{mode === "push" ? "Pushing to Sling…" : "Removing from Sling…"}</h3>
            <p className="muted">
              Batches of 10 with pauses (Sling rate-limits). Don't close this window.
            </p>
            <ProgressBar value={pct} />
            {progress && (
              <p className="muted" style={{ fontVariantNumeric: "tabular-nums" }}>
                {progress.done}/{progress.total} — {progress.created} created, {progress.updated} updated,{" "}
                {progress.deleted} removed
                {progress.failed > 0 && <>, {progress.failed} failed</>}
                {progress.last_label && (
                  <>
                    <br />
                    <code>
                      {progress.last_outcome}: {progress.last_label}
                    </code>
                  </>
                )}
              </p>
            )}
          </>
        )}

        {phase === "done" && summary && (
          <>
            <span className="bk-done-icon">
              <Check size={24} />
            </span>
            <h3 style={{ marginTop: 12 }}>{mode === "push" ? "Sling is up to date" : "Removed from Sling"}</h3>
            <p className="muted" style={{ marginTop: 0 }}>
              {summary.created} created, {summary.updated} updated, {summary.deleted} removed
              {summary.adopted > 0 && <>, {summary.adopted} kept from an earlier draft</>}
              {summary.skipped > 0 && <>, {summary.skipped} skipped</>}
              {summary.failed > 0 && <>, {summary.failed} failed</>}.{" "}
              {mode === "push" && "Open Sling to review and publish."}
            </p>
            {summary.failed > 0 && (
              <p className="muted">Some changes failed — run it again; finished ones are not repeated.</p>
            )}
            {summary.backup_warning && (
              <p className="muted" style={{ color: "var(--color-warning)" }}>
                Heads up: the database backup taken before {mode === "push" ? "pushing" : "removing"} didn't
                save ({summary.backup_warning}). Sling was still updated — try Settings → Backups → Back up now.
              </p>
            )}
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 18 }}>
              <button className="btn-primary" onClick={onClose}>
                Done
              </button>
            </div>
          </>
        )}

        {phase === "error" && (
          <>
            <h3>{title}</h3>
            <div className="error">{error}</div>
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 12 }}>
              <button className="btn-ghost" onClick={onClose}>
                Close
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
