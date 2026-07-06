import { useState } from "react";
import { api } from "../lib/api";

interface Props {
  reason: "first-time" | "expired";
  onSaved: () => void;
  onCancel: () => void;
}

export function SlingTokenModal({ reason, onSaved, onCancel }: Props) {
  const [value, setValue] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const onSave = async () => {
    setError(null);
    setSaving(true);
    try {
      await api.setSlingToken(value);
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const title = reason === "first-time" ? "Add Sling token" : "Sling token expired";
  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        <p className="muted" style={{ marginTop: 0 }}>
          Open <code>app.getsling.com</code> in a browser, log in, open DevTools → Network,
          find any request to <code>api.getsling.com</code>, and copy the value of the
          <code> Authorization</code> header (starts with a long opaque string).
        </p>
        <label className="field">
          <span>Sling Authorization header</span>
          <input
            type="password"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder="eyJ... or opaque bearer string"
            autoFocus
            style={{ fontFamily: "var(--font-mono)" }}
          />
        </label>
        {error && <div className="error">{error}</div>}
        <div className="row" style={{ justifyContent: "space-between", marginTop: 12 }}>
          <button
            className="btn-ghost"
            onClick={async () => {
              try {
                await api.openSlingLoginWindow();
                onCancel(); // close modal; success/cancel toasts come from the Settings listener
              } catch (e) {
                setError(String(e));
              }
            }}
            disabled={saving}
          >
            Log in via Sling
          </button>
          <div className="row">
            <button className="btn-ghost" onClick={onCancel} disabled={saving}>Cancel</button>
            <button className="btn-primary" onClick={onSave} disabled={!value || saving}>
              {saving ? "Saving…" : "Save"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
