// App-level studio auto-setup. Runs Sling detection (async, never blocking)
// after every Sling login / pasted token, on startup while the studio isn't
// configured, and on demand ("Set up studio" / Settings → Detect). The backend
// decides (src-tauri/src/studio_setup.rs):
//   autosaved → small confirmation toast
//   ask       → picker dialog (Later dismisses)
//   ok        → nothing (manual runs still open the picker)
//   mismatch  → non-blocking banner with a Review button (never overwrites)

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import {
  buildPickerModel,
  isStudioComplete,
  notifyStudioConfigChanged,
  onOpenStudioSetup,
  onSlingTokenSet,
  studioSummary,
} from "../lib/studioSetup";
import type { DiscoveredStudio, StudioConfig, StudioDetectOutcome } from "../types";

type Mode = "startup" | "login" | "manual";

interface PickerState {
  found: DiscoveredStudio | null;
  current: StudioConfig;
  error?: string;
}

function friendlyError(msg: string): string {
  if (msg.includes("sling-401")) return "Your Sling login has expired — log in to Sling again (Settings), then retry.";
  if (msg.includes("no Sling token")) return "Log in to Sling first (Settings → Sling token), then retry.";
  return `Couldn't detect your studio from Sling (${msg}). You can enter the IDs in Settings → Studio configuration.`;
}

export function StudioSetup({ onGoSettings }: { onGoSettings: () => void }) {
  const [toast, setToast] = useState<string | null>(null);
  const [mismatch, setMismatch] = useState<StudioDetectOutcome | null>(null);
  const [picker, setPicker] = useState<PickerState | null>(null);
  const [detecting, setDetecting] = useState(false);
  const running = useRef(false);

  const run = useCallback(async (mode: Mode) => {
    if (running.current) return;
    running.current = true;
    if (mode === "manual") setDetecting(true);
    try {
      const out = await api.autoDetectStudioConfig();
      switch (out.decision) {
        case "autosaved":
          setMismatch(null);
          setToast(`Studio set to ${studioSummary(out.discovered, out.current)}`);
          notifyStudioConfigChanged();
          if (mode === "manual") setPicker({ found: out.discovered, current: out.current });
          break;
        case "ask":
          setPicker({ found: out.discovered, current: out.current });
          break;
        case "ok":
          setMismatch(null);
          if (mode === "manual") setPicker({ found: out.discovered, current: out.current });
          break;
        case "mismatch":
          setMismatch(out);
          if (mode === "manual") setPicker({ found: out.discovered, current: out.current });
          break;
      }
    } catch (e) {
      // Automatic runs stay quiet (expired token, offline…); manual runs explain.
      if (mode === "manual") {
        const current = await api.getStudioConfig().catch(
          () => ({ org_id: 0, acting_user_id: 0, home_location_id: 0 }),
        );
        setPicker({ found: null, current, error: friendlyError(String(e)) });
      } else {
        console.warn("[barrekeep] studio auto-detect skipped:", e);
      }
    } finally {
      running.current = false;
      setDetecting(false);
    }
  }, []);

  // Startup: only while the studio isn't configured and a token exists.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [cfg, hasToken] = await Promise.all([api.getStudioConfig(), api.hasSlingToken()]);
        if (!cancelled && hasToken && !isStudioComplete(cfg)) run("startup");
      } catch {
        /* DB not ready etc. — Settings still works manually */
      }
    })();
    return () => { cancelled = true; };
  }, [run]);

  // Every login (browser window or pasted token) and explicit requests.
  useEffect(() => {
    const unlistenTauri = listen<void>("sling-token-saved", () => { run("login"); });
    const offToken = onSlingTokenSet(() => { run("login"); });
    const offOpen = onOpenStudioSetup(() => { run("manual"); });
    return () => {
      unlistenTauri.then((u) => u());
      offToken();
      offOpen();
    };
  }, [run]);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 8000);
    return () => clearTimeout(t);
  }, [toast]);

  return (
    <>
      {detecting && !picker && (
        <div role="status" className="bk-toast">Detecting your studio from Sling…</div>
      )}
      {toast && (
        <div role="status" className="bk-toast">
          <span>{toast} — </span>
          <button className="btn-link" onClick={() => { setToast(null); onGoSettings(); }}>
            change in Settings
          </button>
          <button className="btn-ghost btn-sm" aria-label="Dismiss" onClick={() => setToast(null)}>×</button>
        </div>
      )}
      {mismatch && (
        <div role="status" className="bk-update-banner bk-warn-banner">
          <span style={{ flex: 1 }} title={mismatch.reasons.join("\n")}>
            <strong>Sling login doesn't match the studio configuration.</strong>{" "}
            <span className="muted">{mismatch.reasons[0]}</span>
          </span>
          <button
            className="btn-primary btn-sm"
            onClick={() => setPicker({ found: mismatch.discovered, current: mismatch.current })}
          >
            Review
          </button>
          <button className="btn-ghost btn-sm" onClick={() => setMismatch(null)}>Dismiss</button>
        </div>
      )}
      {picker && (
        <StudioPickerModal
          state={picker}
          onRetry={() => { setPicker(null); run("manual"); }}
          onSaved={(summary) => {
            setPicker(null);
            setMismatch(null);
            setToast(`Studio set to ${summary}`);
            notifyStudioConfigChanged();
          }}
          onLater={() => setPicker(null)}
          onGoSettings={() => { setPicker(null); onGoSettings(); }}
        />
      )}
    </>
  );
}

function StudioPickerModal({ state, onSaved, onLater, onRetry, onGoSettings }: {
  state: PickerState;
  onSaved: (summary: string) => void;
  onLater: () => void;
  onRetry: () => void;
  onGoSettings: () => void;
}) {
  const model = state.found ? buildPickerModel(state.found, state.current) : null;
  const [sel, setSel] = useState<StudioConfig>(
    model?.selected ?? { org_id: 0, acting_user_id: 0, home_location_id: 0 },
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const canSave = !busy && model !== null && isStudioComplete(sel);

  const save = async () => {
    if (!canSave || !state.found) return;
    setBusy(true);
    setError(null);
    try {
      await api.setStudioConfig(sel.org_id, sel.acting_user_id, sel.home_location_id);
      onSaved(studioSummary(state.found, sel));
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const select = (
    label: string,
    key: keyof StudioConfig,
    opts: { id: number; label: string }[],
  ) => (
    <label className="field">
      <span>{label}</span>
      {opts.length === 1 && sel[key] === opts[0].id ? (
        <div style={{ padding: "6px 0" }}>{opts[0].label}</div>
      ) : (
        <select
          value={String(sel[key] || "")}
          onChange={(e) => setSel({ ...sel, [key]: Number(e.target.value) })}
          disabled={busy}
        >
          <option value="">— choose —</option>
          {opts.map((o) => <option key={o.id} value={String(o.id)}>{o.label}</option>)}
        </select>
      )}
    </label>
  );

  return (
    <div className="modal-backdrop" onClick={busy ? undefined : onLater}>
      <div className="modal" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Set up your studio">
        <h3>Set up your studio</h3>
        {model ? (
          <>
            <p className="muted" style={{ marginTop: 0 }}>
              Found in your Sling login. Pulls and pushes will target this studio;
              other locations are ignored.
            </p>
            <div style={{ display: "grid", gap: 10 }}>
              {select("Organization", "org_id", model.orgs)}
              {select("Acting user (admin calendar feed)", "acting_user_id", model.users)}
              {model.locations.length > 0
                ? select("Home location", "home_location_id", model.locations)
                : <div className="muted">Sling reported no locations for this login — enter the location id in Settings.</div>}
            </div>
          </>
        ) : (
          <div className="error">{state.error}</div>
        )}
        {error && <div className="error">{error}</div>}
        <div className="row" style={{ justifyContent: "flex-end", marginTop: 18 }}>
          <button className="btn-ghost" onClick={onGoSettings} disabled={busy} style={{ marginRight: "auto" }}>
            Open Settings
          </button>
          {!model && <button className="btn-ghost" onClick={onRetry}>Retry</button>}
          <button className="btn-ghost" onClick={onLater} disabled={busy}>Later</button>
          {model && (
            <button className="btn-primary" onClick={save} disabled={!canSave}>
              {busy ? "Saving…" : "Save"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
