import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { checkForUpdatesOnStartup } from "./lib/updater";
import "./styles.css";
import "./components/calendar/calendar.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

void checkForUpdatesOnStartup();
