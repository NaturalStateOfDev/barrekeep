// Self-update helpers over tauri-plugin-updater + tauri-plugin-process.
//
// Flow: check() hits the GitHub Releases endpoint in tauri.conf.json
// (latest.json), compares the installed version against the latest signed
// release, and returns an Update handle if a newer one exists. installUpdate()
// downloads (signature-verified against the pubkey in tauri.conf.json),
// installs, and relaunches.
//
// Until a signing keypair is configured (pubkey in tauri.conf.json + a signed
// release published by .github/workflows/release.yml), and in dev, check()
// will return null or throw — callers treat that as "no update / can't check".

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

export type { Update };

/** The installed app version (from tauri.conf.json). */
export function getCurrentVersion(): Promise<string> {
  return getVersion();
}

/**
 * Returns an Update handle if a newer signed release is available, else null.
 * May throw on network/endpoint/signature errors — callers decide how loud to
 * be (startup is silent; the manual check surfaces the error).
 */
export function checkForUpdate(): Promise<Update | null> {
  return check();
}

export interface DownloadProgress {
  downloaded: number;
  total: number | null;
  /** 0–100, or null when the server didn't send a content length. */
  percent: number | null;
}

/**
 * Download + install the given update (reporting progress), then relaunch into
 * the new version. If this resolves without throwing, the app is restarting.
 */
export async function installUpdate(
  update: Update,
  onProgress?: (p: DownloadProgress) => void,
): Promise<void> {
  let downloaded = 0;
  let total: number | null = null;

  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? null;
        onProgress?.({ downloaded: 0, total, percent: total ? 0 : null });
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        onProgress?.({
          downloaded,
          total,
          percent: total ? Math.min(100, Math.round((downloaded / total) * 100)) : null,
        });
        break;
      case "Finished":
        onProgress?.({ downloaded, total, percent: 100 });
        break;
    }
  });

  await relaunch();
}
