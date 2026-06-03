// Auto-update check on app startup.
//
// Flow:
//   1. Hit the endpoint in tauri.conf.json (GitHub Releases /latest/download/latest.json)
//   2. If a newer signed version exists, ask the user
//   3. Download, install, relaunch
//
// In dev mode the check fails silently (no signed artifacts to fetch).

import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export async function checkForUpdatesOnStartup(): Promise<void> {
  try {
    const update = await check();
    if (!update) return;

    const accept = window.confirm(
      `Update available: ${update.version}\n\n${update.body ?? ""}\n\nInstall now? The app will restart.`,
    );
    if (!accept) return;

    await update.downloadAndInstall();
    await relaunch();
  } catch (err) {
    // Network errors, missing endpoint in dev, malformed manifest — all
    // non-fatal. Log and move on; the user can still use the app.
    console.warn("Update check failed:", err);
  }
}
