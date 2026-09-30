// Studio auto-setup plumbing shared by the App-level <StudioSetup> component,
// Settings, and the screens whose Sling actions can fail with "Studio not
// configured". The decision rule itself lives in the backend
// (src-tauri/src/studio_setup.rs) — this file only carries window events and
// the picker's option/preselection logic.

import type { DiscoveredStudio, StudioConfig } from "../types";

const EV_TOKEN_SET = "barrekeep:sling-token-set";
const EV_OPEN_SETUP = "barrekeep:open-studio-setup";
const EV_CONFIG_CHANGED = "barrekeep:studio-config-changed";

function on(name: string, fn: () => void): () => void {
  window.addEventListener(name, fn);
  return () => window.removeEventListener(name, fn);
}

/** A Sling token was pasted/saved from the frontend (the login window emits its own Tauri event). */
export const notifySlingTokenSet = () => window.dispatchEvent(new Event(EV_TOKEN_SET));
export const onSlingTokenSet = (fn: () => void) => on(EV_TOKEN_SET, fn);

/** Ask <StudioSetup> to detect from Sling and show the picker. */
export const openStudioSetup = () => window.dispatchEvent(new Event(EV_OPEN_SETUP));
export const onOpenStudioSetup = (fn: () => void) => on(EV_OPEN_SETUP, fn);

/** studio_config was saved; Settings refreshes its fields. */
export const notifyStudioConfigChanged = () => window.dispatchEvent(new Event(EV_CONFIG_CHANGED));
export const onStudioConfigChanged = (fn: () => void) => on(EV_CONFIG_CHANGED, fn);

/** True for the backend's "Studio not configured — …" errors. */
export function isStudioNotConfigured(message: string): boolean {
  return /studio not configured/i.test(message);
}

export function isStudioComplete(c: StudioConfig): boolean {
  return c.org_id > 0 && c.acting_user_id > 0 && c.home_location_id > 0;
}

export interface PickerOption {
  id: number;
  label: string;
}

export interface PickerModel {
  orgs: PickerOption[];
  users: PickerOption[];
  locations: PickerOption[];
  /** Preselected ids (0 = nothing chosen). */
  selected: StudioConfig;
}

/**
 * Options for the studio picker: what Sling detected, plus the currently
 * configured value when Sling didn't report it (so "Review" can show it and
 * keeping it stays possible). Preselects current values when set, otherwise a
 * sole candidate.
 */
export function buildPickerModel(found: DiscoveredStudio, current: StudioConfig): PickerModel {
  const withCurrent = (opts: PickerOption[], cur: number, what: string): PickerOption[] =>
    cur > 0 && !opts.some((o) => o.id === cur)
      ? [...opts, { id: cur, label: `Currently configured ${what} (${cur}) — not visible to this login` }]
      : opts;
  const orgs = withCurrent(
    [{ id: found.org_id, label: found.org_name ? `${found.org_name} (${found.org_id})` : `Organization ${found.org_id}` }],
    current.org_id,
    "organization",
  );
  const users = withCurrent(
    [{ id: found.acting_user_id, label: found.acting_user_name ? `${found.acting_user_name} (${found.acting_user_id})` : `User ${found.acting_user_id}` }],
    current.acting_user_id,
    "user",
  );
  const locations = withCurrent(
    found.locations.map((l) => ({ id: l.id, label: l.name || `Location ${l.id}` })),
    current.home_location_id,
    "location",
  );
  const pick = (opts: PickerOption[], cur: number) =>
    cur > 0 ? cur : opts.length === 1 ? opts[0].id : 0;
  return {
    orgs,
    users,
    locations,
    selected: {
      org_id: pick(orgs, current.org_id),
      acting_user_id: pick(users, current.acting_user_id),
      home_location_id: pick(locations, current.home_location_id),
    },
  };
}

/** "Studio set to <org> · <location>" confirmation text. */
export function studioSummary(found: DiscoveredStudio, cfg: StudioConfig): string {
  const org = found.org_id === cfg.org_id && found.org_name ? found.org_name : `org ${cfg.org_id}`;
  const loc = found.locations.find((l) => l.id === cfg.home_location_id)?.name ?? `location ${cfg.home_location_id}`;
  return `${org} · ${loc}`;
}
