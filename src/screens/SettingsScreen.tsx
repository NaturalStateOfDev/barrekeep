import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ExternalLink, Radar } from "lucide-react";
import { api } from "../lib/api";
import { SlingTokenModal } from "../components/SlingTokenModal";
import { PageHead } from "../components/ui/PageHead";
import { Field } from "../components/ui/Field";
import {
  getCurrentVersion,
  checkForUpdate,
  installUpdate,
  type Update,
  type DownloadProgress,
} from "../lib/updater";
import type { DbInfo, DiscoveredLocation } from "../types";

function StatusValue({ state, okLabel, warnLabel, mutedLabel }: {
  state: boolean | null;
  okLabel: string;
  warnLabel?: string;
  mutedLabel: string;
}) {
  if (state === null) return <span className="muted">checking…</span>;
  if (state) return <span style={{ color: "var(--color-success)", fontWeight: 600 }}>{okLabel}</span>;
  if (warnLabel) return <span style={{ color: "var(--color-warning)", fontWeight: 600 }}>{warnLabel}</span>;
  return <span className="muted">{mutedLabel}</span>;
}

export function SettingsScreen() {
  return (
    <div>
      <PageHead title="Settings" sub="Sling connection, studio identifiers & Claude review." />
      <div style={{ display: "flex", flexDirection: "column", maxWidth: 620 }}>
        <SlingTokenCard />
        <StudioConfigCard />
        <AnthropicKeyCard />
        <SlingCredentialsCard />
        <UpdatesCard />
        <DatabaseCard />
      </div>
    </div>
  );
}

function SlingTokenCard() {
  const [hasToken, setHasToken] = useState<boolean | null>(null);
  const [showModal, setShowModal] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const refresh = () => api.hasSlingToken().then(setHasToken).catch((e) => setError(String(e)));

  useEffect(() => { refresh(); }, []);

  useEffect(() => {
    const unsubs: Array<Promise<() => void>> = [];
    unsubs.push(listen<void>("sling-token-saved", () => {
      setToast("Logged in to Sling.");
      refresh();
    }));
    unsubs.push(listen<void>("sling-login-cancelled", () => {
      setToast("Sign-in cancelled.");
    }));
    return () => {
      unsubs.forEach((p) => p.then((u) => u()));
    };
  }, []);

  const onLoginBrowser = async () => {
    setError(null);
    setToast(null);
    try {
      await api.openSlingLoginWindow();
    } catch (e) {
      setError(String(e));
    }
  };

  const onClear = async () => {
    try {
      await api.setSlingToken("");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="card">
      <strong>Sling token</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Required for "Pull from Sling". Stored in OS keychain (Stronghold); survives
        app restarts. If a pull returns 401, you'll be prompted to paste a fresh one.
      </p>
      <div style={{ marginTop: 12 }}>
        Status: <StatusValue state={hasToken} okLabel="set" mutedLabel="not set" />
      </div>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={() => setShowModal(true)}>
          {hasToken ? "Update" : "Set token"}
        </button>
        <button className="btn-ghost" onClick={onLoginBrowser}>
          <ExternalLink size={15} /> Log in via Sling
        </button>
        {hasToken && <button className="btn-ghost" onClick={onClear}>Clear</button>}
      </div>
      {error && <div className="error">{error}</div>}
      {toast && <div className="ok">{toast}</div>}
      {showModal && (
        <SlingTokenModal
          reason="first-time"
          onSaved={() => { setShowModal(false); refresh(); }}
          onCancel={() => setShowModal(false)}
        />
      )}
    </div>
  );
}

function StudioConfigCard() {
  const [orgId, setOrgId] = useState("");
  const [actingUserId, setActingUserId] = useState("");
  const [homeLocationId, setHomeLocationId] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [locations, setLocations] = useState<DiscoveredLocation[]>([]);
  const [detecting, setDetecting] = useState(false);
  const [detectMsg, setDetectMsg] = useState<string | null>(null);

  const refresh = () =>
    api.getStudioConfig().then((c) => {
      setOrgId(String(c.org_id));
      setActingUserId(String(c.acting_user_id));
      setHomeLocationId(String(c.home_location_id));
      setLoaded(true);
    }).catch((e) => setError(String(e)));

  const detect = async () => {
    setDetecting(true);
    setDetectMsg(null);
    setError(null);
    try {
      const d = await api.discoverStudioConfig();
      setOrgId(String(d.org_id));
      setActingUserId(String(d.acting_user_id));
      setLocations(d.locations);
      if (d.locations.length === 1) setHomeLocationId(String(d.locations[0].id));
      setDetectMsg(
        `Detected ${d.acting_user_name || "your"} studio (org ${d.org_id}). ` +
        `Pick the home location and click Save.`,
      );
    } catch (e) {
      const msg = String(e);
      if (msg.includes("sling-401")) {
        setDetectMsg("Sling token expired — use “Log in to Sling” above to refresh, then Detect again.");
      } else if (msg.includes("no Sling token")) {
        setDetectMsg("Log in to Sling first (card above), then Detect.");
      } else {
        setDetectMsg("Couldn't auto-detect everything — enter the IDs manually below.");
      }
    } finally {
      setDetecting(false);
    }
  };

  useEffect(() => { refresh(); }, []);

  useEffect(() => {
    const p = listen<void>("sling-token-saved", () => { detect(); });
    return () => { p.then((un) => un()); };
  }, []);

  const configured = loaded && Number(orgId) > 0 && Number(homeLocationId) > 0;

  const onSave = async () => {
    setError(null);
    setStatus(null);
    const o = Number(orgId), a = Number(actingUserId), h = Number(homeLocationId);
    if (![o, a, h].every((n) => Number.isInteger(n) && n >= 0)) {
      setError("All three IDs must be non-negative whole numbers.");
      return;
    }
    try {
      await api.setStudioConfig(o, a, h);
      setStatus("Saved. Pulls will now target this studio.");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const mono = { fontFamily: "var(--font-mono)" } as const;

  return (
    <div className="card">
      <strong>Studio configuration</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Your studio's Sling identifiers. Required before pulling. Find them in a
        Sling DevTools session: the <code>org id</code> and admin{" "}
        <code>acting-user id</code> appear in the calendar request URL, and the{" "}
        <code>home location id</code> is your studio's location (other locations
        are filtered out). Stored locally in this app's database only.
      </p>
      <div style={{ marginTop: 12 }}>
        Status:{" "}
        {!loaded ? <span className="muted">checking…</span>
          : configured ? <span style={{ color: "var(--color-success)", fontWeight: 600 }}>configured</span>
          : <span style={{ color: "var(--color-warning)", fontWeight: 600 }}>not configured — pulls disabled</span>}
      </div>
      <div style={{ display: "grid", gap: 10, marginTop: 12 }}>
        <Field label="Organization id">
          <input type="number" min={0} value={orgId} onChange={(e) => setOrgId(e.target.value)} placeholder="0" style={mono} />
        </Field>
        <Field label="Acting-user id (admin calendar feed)">
          <input type="number" min={0} value={actingUserId} onChange={(e) => setActingUserId(e.target.value)} placeholder="0" style={mono} />
        </Field>
        <Field label={locations.length > 0 ? "Home location" : "Home location id"}>
          {locations.length > 0 ? (
            <select value={homeLocationId} onChange={(e) => setHomeLocationId(e.target.value)} style={mono}>
              <option value="">— pick your studio —</option>
              {locations.map((l) => (
                <option key={l.id} value={String(l.id)}>{l.name} ({l.id})</option>
              ))}
            </select>
          ) : (
            <input type="number" min={0} value={homeLocationId} onChange={(e) => setHomeLocationId(e.target.value)} placeholder="0" style={mono} />
          )}
        </Field>
      </div>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={onSave}>Save</button>
        <button className="btn-ghost" onClick={detect} disabled={detecting}>
          <Radar size={15} /> {detecting ? "Detecting…" : "Detect from Sling"}
        </button>
      </div>
      {detectMsg && <div className="muted" style={{ marginTop: 8 }}>{detectMsg}</div>}
      {status && <div className="ok">{status}</div>}
      {error && <div className="error">{error}</div>}
    </div>
  );
}

const CLAUDE_MODEL_OPTIONS = [
  { id: "claude-opus-4-8", label: "Claude Opus 4.8 — most capable (~13¢ per interaction)" },
  { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6 — balanced (~7¢ per interaction)" },
  { id: "claude-haiku-4-5", label: "Claude Haiku 4.5 — cheapest (~2–3¢ per interaction)" },
];
const DEFAULT_CLAUDE_MODEL = "claude-opus-4-8";

function AnthropicKeyCard() {
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [model, setModel] = useState<string>(DEFAULT_CLAUDE_MODEL);

  useEffect(() => {
    api.hasAnthropicKey().then(setHasKey).catch((e) => setError(String(e)));
    api.getAppSetting("claude_model")
      .then((m) => { if (m) setModel(m); })
      .catch(() => {});
  }, []);

  const onModelChange = async (next: string) => {
    setModel(next);
    try {
      await api.setAppSetting("claude_model", next);
    } catch (e) {
      setError(String(e));
    }
  };

  const onSave = async () => {
    setError(null);
    setStatus(null);
    try {
      await api.setAnthropicKey(keyInput);
      setKeyInput("");
      const has = await api.hasAnthropicKey();
      setHasKey(has);
      setStatus(has ? "Saved to the OS keychain — survives restarts." : "Cleared.");
    } catch (e) {
      setError(String(e));
    }
  };

  const onClear = async () => {
    try {
      await api.setAnthropicKey("");
      setHasKey(false);
      setStatus("Cleared.");
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="card">
      <strong>Anthropic API key</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Required for the Claude features on proposals (review, editing,
        algorithm updates). Stored in the OS keychain (Stronghold) — survives
        restarts. Get a key from <code>console.anthropic.com</code>.
      </p>
      <div style={{ marginTop: 12 }}>
        Status: <StatusValue state={hasKey} okLabel="set" mutedLabel="not set" />
      </div>
      <Field label="Model" style={{ marginTop: 12 }} hint="Used by review, the proposal editor, and code drafting.">
        <select value={model} onChange={(e) => onModelChange(e.target.value)}>
          {CLAUDE_MODEL_OPTIONS.map((o) => (
            <option key={o.id} value={o.id}>{o.label}</option>
          ))}
        </select>
      </Field>
      <Field label="Paste key" style={{ marginTop: 12 }}>
        <input
          type="password"
          value={keyInput}
          onChange={(e) => setKeyInput(e.target.value)}
          placeholder="sk-ant-..."
          style={{ fontFamily: "var(--font-mono)" }}
        />
      </Field>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={onSave} disabled={!keyInput}>
          Save
        </button>
        {hasKey && (
          <button className="btn-ghost" onClick={onClear}>
            Clear
          </button>
        )}
      </div>
      {status && <div className="ok">{status}</div>}
      {error && <div className="error">{error}</div>}
    </div>
  );
}

function SlingCredentialsCard() {
  const [hasCreds, setHasCreds] = useState<boolean | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    api.hasSlingCredentials().then(setHasCreds).catch((e) => setError(String(e)));

  useEffect(() => { refresh(); }, []);

  const onSave = async () => {
    setError(null);
    setStatus(null);
    if (!email.trim()) {
      setError("Email is required.");
      return;
    }
    try {
      await api.setSlingCredentials(email.trim(), password);
      setEmail("");
      setPassword("");
      setStatus("Saved. Sling login form will be pre-filled next time.");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onClear = async () => {
    setError(null);
    setStatus(null);
    try {
      await api.setSlingCredentials("", "");
      setStatus("Cleared.");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="card">
      <strong>Sling login credentials (optional)</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Saved in OS keychain (Stronghold) and used only to pre-fill Sling's
        login form when you click "Log in via Sling". Captcha and the submit
        click stay with you. Leave blank if you'd rather type them each time.
      </p>
      <div style={{ marginTop: 12 }}>
        Status: <StatusValue state={hasCreds} okLabel="saved" mutedLabel="not saved" />
      </div>
      <div style={{ display: "grid", gap: 10, marginTop: 12, maxWidth: 360 }}>
        <Field label="Email">
          <input
            type="email"
            autoComplete="off"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
        </Field>
        <Field label="Password">
          <input
            type="password"
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </Field>
      </div>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={onSave}>
          {hasCreds ? "Update" : "Save"}
        </button>
        {hasCreds && (
          <button className="btn-ghost" onClick={onClear}>Clear</button>
        )}
      </div>
      {status && <div className="ok">{status}</div>}
      {error && <div className="error">{error}</div>}
    </div>
  );
}

function UpdatesCard() {
  const [version, setVersion] = useState<string>("");
  const [update, setUpdate] = useState<Update | null>(null);
  const [state, setState] = useState<
    "idle" | "checking" | "current" | "available" | "installing" | "error"
  >("idle");
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getCurrentVersion().then(setVersion).catch(() => {});
  }, []);

  const onCheck = async () => {
    setState("checking");
    setError(null);
    try {
      const u = await checkForUpdate();
      if (u) {
        setUpdate(u);
        setState("available");
      } else {
        setState("current");
      }
    } catch (e) {
      setState("error");
      setError(String(e));
    }
  };

  const onInstall = async () => {
    if (!update) return;
    setState("installing");
    setError(null);
    try {
      await installUpdate(update, setProgress);
      // relaunches on success
    } catch (e) {
      setState("error");
      setError(String(e));
    }
  };

  const pct = progress?.percent;

  return (
    <div className="card">
      <strong>Updates</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Barrekeep checks for a newer signed release on startup and installs it
        with your approval. You can also check on demand here.
      </p>
      <div style={{ marginTop: 12 }}>
        Current version:{" "}
        {version ? <code>v{version}</code> : <span className="muted">…</span>}
      </div>

      <div className="row" style={{ marginTop: 12 }}>
        {state === "available" ? (
          <button className="btn-primary" onClick={onInstall}>
            Install v{update?.version} &amp; restart
          </button>
        ) : (
          <button
            className="btn-primary"
            onClick={onCheck}
            disabled={state === "checking" || state === "installing"}
          >
            {state === "checking" ? "Checking…" : "Check for updates"}
          </button>
        )}
      </div>

      {state === "current" && (
        <div className="ok" style={{ marginTop: 8 }}>You're on the latest version.</div>
      )}
      {state === "available" && (
        <div className="ok" style={{ marginTop: 8 }}>
          v{update?.version} is ready to install.
        </div>
      )}
      {state === "installing" && (
        <div className="muted" style={{ marginTop: 8 }}>
          Downloading{pct != null ? ` ${pct}%` : "…"} — the app will restart when done.
        </div>
      )}
      {state === "error" && (
        <div className="error" style={{ marginTop: 8 }}>
          Couldn't check for updates: {error}
        </div>
      )}
    </div>
  );
}

function DatabaseCard() {
  const [info, setInfo] = useState<DbInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.dbInfo().then(setInfo).catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="card">
      <strong>Database</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        Local DuckDB file — schedule history, roster and pulls all live here.
      </p>
      {error && <div className="error">{error}</div>}
      {info && (
        <table style={{ marginTop: 8 }}>
          <tbody>
            <tr>
              <td className="muted">Path</td>
              <td>
                <code>{info.path}</code>
              </td>
            </tr>
            <tr>
              <td className="muted">Schema version</td>
              <td>{info.schema_version}</td>
            </tr>
            <tr>
              <td className="muted">Teachers</td>
              <td>{info.teacher_count}</td>
            </tr>
            <tr>
              <td className="muted">Class types</td>
              <td>{info.position_count}</td>
            </tr>
          </tbody>
        </table>
      )}
    </div>
  );
}
