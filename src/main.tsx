import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";

// Barre & Bloom webfonts, bundled (no CDN — the app must work offline).
// Newsreader (serif display; italic for the wordmark) + Hanken Grotesk (body).
import "@fontsource/newsreader/400.css";
import "@fontsource/newsreader/500.css";
import "@fontsource/newsreader/600.css";
import "@fontsource/newsreader/700.css";
import "@fontsource/newsreader/600-italic.css";
import "@fontsource/hanken-grotesk/400.css";
import "@fontsource/hanken-grotesk/500.css";
import "@fontsource/hanken-grotesk/600.css";
import "@fontsource/hanken-grotesk/700.css";

import "./styles.css";
import "./components/calendar/calendar.css";

async function start() {
  // Browser-only preview (npm run dev outside Tauri): install a mock IPC
  // layer with demo data so the UI is viewable without the Rust shell.
  // Dead-code-eliminated from production builds.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installDevMock } = await import("./lib/devMock");
    installDevMock();
  }

  // The startup update check now lives in <UpdateBanner /> (rendered by App),
  // which surfaces an available update as an in-app banner instead of a
  // blocking dialog.
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

start();
