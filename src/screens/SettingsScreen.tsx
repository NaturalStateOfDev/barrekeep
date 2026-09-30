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
import type { BackupsInfo, DbInfo, PythonStatus } from "../types";
import { onStudioConfigChanged, openStudioSetup } from "../lib/studioSetup";
import { useTimeFormat } from "../lib/timeFormat";

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
        <DisplayCard />
        <AnthropicKeyCard />
        <SlingCredentialsCard />
        <UpdatesCard />
        <PythonCard />
        <DatabaseCard />
        <BackupsCard />
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

  const refresh = () =>
    api.getStudioConfig().then((c) => {
      setOrgId(String(c.org_id));
      setActingUserId(String(c.acting_user_id));
      setHomeLocationId(String(c.home_location_id));
      setLoaded(true);
    }).catch((e) => setError(String(e)));

  useEffect(() => { refresh(); }, []);
  // Auto-detection (App-level <StudioSetup>) saves here too — reload.
  useEffect(() => onStudioConfigChanged(() => { refresh(); }), []);

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
        Your studio's Sling identifiers. Required before pulling. Detected
        automatically when you log in to Sling; otherwise find them in a
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
        <Field label="Home location id">
          <input type="number" min={0} value={homeLocationId} onChange={(e) => setHomeLocationId(e.target.value)} placeholder="0" style={mono} />
        </Field>
      </div>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={onSave}>Save</button>
        <button className="btn-ghost" onClick={openStudioSetup}>
          <Radar size={15} /> Detect from Sling
        </button>
      </div>
      {status && <div className="ok">{status}</div>}
      {error && <div className="error">{error}</div>}
    </div>
  );
}

function DisplayCard() {
  const tf = useTimeFormat();
  const [error, setError] = useState<string | null>(null);
  const onToggle = async (use24: boolean) => {
    setError(null);
    try {
      await tf.setFormat(use24 ? "24h" : "12h");
    } catch (e) {
      setError(String(e));
    }
  };
  return (
    <div className="card">
      <strong>Display</strong>
      <label className="row" style={{ marginTop: 12, gap: 8, cursor: "pointer" }}>
        <input
          type="checkbox"
          checked={tf.fmt === "24h"}
          onChange={(e) => onToggle(e.target.checked)}
        />
        Use 24-hour time
      </label>
      <p className="muted" style={{ marginTop: 6, marginBottom: 0 }}>
        Class times show as {tf.time("17:30")} ({tf.range("09:45", "10:35")}). Display only —
        Sling, rules and Claude always use 24-hour times.
      </p>
      {error && <div className="error">{error}</div>}
    </div>
  );
}

// Keep in sync with CLAUDE_MODELS in src-tauri/src/commands.rs. A stored id
// that isn't listed (e.g. a retired model saved by an older build) shows as
// the default — the backend falls back to it the same way.
const CLAUDE_MODEL_OPTIONS = [
  { id: "claude-opus-5-5", label: "Claude Opus 5.5 — most capable (~10¢ per interaction)" },
  { id: "claude-sonnet-5-5", label: "Claude Sonnet 5.5 — balanced (~5¢ per interaction)" },
  { id: "claude-haiku-4-5", label: "Claude Haiku 4.5 — cheapest (~2–3¢ per interaction)" },
];
const DEFAULT_CLAUDE_MODEL = "claude-opus-5-5";

function AnthropicKeyCard() {
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [keyInput, setKeyInput] = useState("");
  // Keychain writes take a second or more; block double-submits meanwhile.
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [model, setModel] = useState<string>(DEFAULT_CLAUDE_MODEL);

  useEffect(() => {
    api.hasAnthropicKey().then(setHasKey).catch((e) => setError(String(e)));
    api.getAppSetting("claude_model")
      .then((m) => {
        if (m && CLAUDE_MODEL_OPTIONS.some((o) => o.id === m)) setModel(m);
      })
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
    if (saving) return;
    setSaving(true);
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
    } finally {
      setSaving(false);
    }
  };

  const onClear = async () => {
    if (saving) return;
    setSaving(true);
    try {
      await api.setAnthropicKey("");
      setHasKey(false);
      setStatus("Cleared.");
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
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
        <button className="btn-primary" onClick={onSave} disabled={!keyInput || saving}>
          {saving ? "Saving…" : "Save"}
        </button>
        {hasKey && (
          <button className="btn-ghost" onClick={onClear} disabled={saving}>
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
  const [saving, setSaving] = useState(false);
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
    if (saving) return;
    setSaving(true);
    try {
      await api.setSlingCredentials(email.trim(), password);
      setEmail("");
      setPassword("");
      setStatus("Saved. Sling login form will be pre-filled next time.");
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const onClear = async () => {
    setError(null);
    setStatus(null);
    if (saving) return;
    setSaving(true);
    try {
      await api.setSlingCredentials("", "");
      setStatus("Cleared.");
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
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
        <button className="btn-primary" onClick={onSave} disabled={saving}>
          {saving ? "Saving…" : hasCreds ? "Update" : "Save"}
        </button>
        {hasCreds && (
          <button className="btn-ghost" onClick={onClear} disabled={saving}>Clear</button>
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

function PythonCard() {
  const [status, setStatus] = useState<PythonStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const check = async () => {
    setChecking(true);
    setError(null);
    try {
      setStatus(await api.checkPython());
    } catch (e) {
      setError(String(e));
    } finally {
      setChecking(false);
    }
  };

  useEffect(() => { check(); }, []);

  return (
    <div className="card">
      <strong>Python</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        "Generate proposal" runs the schedule algorithm (<code>propose.py</code>)
        with Python {status?.min_version ?? "3.11"} or newer.
      </p>
      <div style={{ marginTop: 12 }}>
        Status:{" "}
        {checking && !status ? <span className="muted">checking…</span>
          : status?.found ? (
            <span style={{ color: "var(--color-success)", fontWeight: 600 }}>
              Python {status.version}
            </span>
          ) : status ? (
            <span style={{ color: "var(--color-warning)", fontWeight: 600 }}>not found</span>
          ) : <span className="muted">unknown</span>}
      </div>
      {status?.found && (
        <table style={{ marginTop: 8 }}>
          <tbody>
            <tr>
              <td className="muted">Command</td>
              <td><code>{status.command}</code></td>
            </tr>
            {status.path && (
              <tr>
                <td className="muted">Path</td>
                <td><code>{status.path}</code></td>
              </tr>
            )}
          </tbody>
        </table>
      )}
      {status && !status.found && status.error && (
        <div className="error" style={{ marginTop: 8 }}>{status.error}</div>
      )}
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-ghost" onClick={check} disabled={checking}>
          {checking ? "Checking…" : "Re-check"}
        </button>
      </div>
      {error && <div className="error">{error}</div>}
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

const BACKUP_REASON_LABELS: Record<string, string> = {
  startup: "daily (startup)",
  prepush: "before Sling push",
  preremove: "before removing from Sling",
  manual: "manual",
};

function BackupsCard() {
  const tf = useTimeFormat();
  const [info, setInfo] = useState<BackupsInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => api.listBackups().then(setInfo).catch((e) => setError(String(e)));

  useEffect(() => { refresh(); }, []);

  const onBackupNow = async () => {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      const b = await api.backupNow();
      setStatus(`Backed up to ${b.name}.`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      refresh();
    }
  };

  const onOpenFolder = async () => {
    setError(null);
    try {
      await api.openBackupsFolder();
    } catch (e) {
      setError(`${e} — copy the path instead.`);
    }
  };

  const onCopyPath = async () => {
    if (!info) return;
    setError(null);
    try {
      await navigator.clipboard.writeText(info.dir);
      setStatus("Folder path copied.");
    } catch {
      setError("Couldn't copy — select the path and copy it manually.");
    }
  };

  return (
    <div className="card">
      <strong>Backups</strong>
      <p className="muted" style={{ marginTop: 4 }}>
        A copy of the database is saved once a day when Barrekeep starts and
        before every Sling push; the newest {info?.keep ?? 14} are kept. To
        restore one, see <code>docs/architecture.md</code> → "Restoring a backup".
      </p>
      {info?.last_error && (
        <div className="error" style={{ marginTop: 8 }}>
          Last backup failed: {info.last_error}
        </div>
      )}
      {info && (
        <div style={{ marginTop: 12 }}>
          <span className="muted">Folder </span>
          <code style={{ wordBreak: "break-all" }}>{info.dir}</code>
        </div>
      )}
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn-primary" onClick={onBackupNow} disabled={busy}>
          {busy ? "Backing up…" : "Back up now"}
        </button>
        <button className="btn-ghost" onClick={onOpenFolder}>Open backups folder</button>
        <button className="btn-ghost" onClick={onCopyPath} disabled={!info}>Copy path</button>
      </div>
      {status && <div className="ok">{status}</div>}
      {error && <div className="error">{error}</div>}
      {info && info.backups.length === 0 && (
        <div className="muted" style={{ marginTop: 8 }}>No backups yet.</div>
      )}
      {info && info.backups.length > 0 && (
        <table style={{ marginTop: 8 }}>
          <thead>
            <tr>
              <th style={{ textAlign: "left" }}>Taken</th>
              <th style={{ textAlign: "left" }}>Why</th>
              <th style={{ textAlign: "right" }}>Size</th>
            </tr>
          </thead>
          <tbody>
            {info.backups.map((b) => (
              <tr key={b.name} title={b.name}>
                <td>{tf.timestamp(b.created_at)}</td>
                <td className="muted">{BACKUP_REASON_LABELS[b.reason.replace(/\d+$/, "")] ?? b.reason}</td>
                <td style={{ textAlign: "right" }}>{formatBytes(b.size_bytes)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
