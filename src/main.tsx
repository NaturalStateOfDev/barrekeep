import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import "./styles.css";
import "./components/calendar/calendar.css";

// The startup update check now lives in <UpdateBanner /> (rendered by App),
// which surfaces an available update as an in-app banner instead of a
// blocking dialog.
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
