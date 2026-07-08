import { Component, type ErrorInfo, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

// Persist a frontend error to the Rust-side log file (best-effort). Used by
// the boundary below and by the global handlers in main.tsx.
export function logFrontendError(message: string): void {
  invoke("log_frontend_error", { message }).catch(() => {});
}

interface Props {
  children: ReactNode;
}
interface State {
  error: Error | null;
}

/** Catches render/lifecycle errors anywhere in the tree so a thrown component
 *  shows a readable message (and gets logged) instead of a blank white window,
 *  which is otherwise impossible to diagnose in a release build. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    logFrontendError(
      `render error: ${error.message}\n${error.stack ?? ""}\ncomponentStack:${info.componentStack ?? ""}`,
    );
  }

  render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div style={{ padding: 24, maxWidth: 900, margin: "0 auto" }}>
        <h2 style={{ fontFamily: "sans-serif" }}>
          Barrekeep hit an error and couldn’t display this screen
        </h2>
        <p style={{ fontFamily: "sans-serif", color: "#444" }}>{error.message}</p>
        <pre
          style={{
            fontFamily: "ui-monospace, monospace",
            fontSize: 12,
            lineHeight: 1.5,
            overflow: "auto",
            background: "#f5f5f5",
            color: "#333",
            padding: 12,
            borderRadius: 6,
          }}
        >
          {error.stack}
        </pre>
        <p style={{ fontFamily: "sans-serif", fontSize: 13, color: "#666" }}>
          Details were saved to the log file at{" "}
          <code>%LOCALAPPDATA%\com.barrekeep.app\logs\barrekeep.log</code>.
        </p>
      </div>
    );
  }
}
