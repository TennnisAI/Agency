import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, getStoredTheme } from "./lib/themes";
import { toastError } from "./lib/toast";
import "./theme.css";
import "./styles.css";

applyTheme(getStoredTheme());

// Suppress the webview's default right-click menu (Reload / Developer Tools),
// which is irrelevant to end users. Terminals and editable text fields keep
// their native menu so copy/paste still works there.
window.addEventListener("contextmenu", (e) => {
  const target = e.target as HTMLElement | null;
  if (target?.closest("input, textarea, [contenteditable=\"true\"], .terminal, .xterm")) return;
  e.preventDefault();
});

// Surface errors that escape every component handler. The toast bus dedupes
// identical messages, so a rejecting poll loop can't stack toasts.
window.addEventListener("error", (e) => {
  toastError(e.error ?? e.message, "Unexpected error");
});
window.addEventListener("unhandledrejection", (e) => {
  toastError(e.reason, "Unexpected error");
});

// Last-resort boundary: a render throw otherwise unmounts the whole tree and
// leaves a blank window. Styled inline because theme CSS variables may be the
// very thing that failed to load.
class RootErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return (
        <div
          style={{
            height: "100vh",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            gap: 12,
            background: "#11111b",
            color: "#cdd6f4",
            fontFamily: "-apple-system, BlinkMacSystemFont, sans-serif",
            padding: 32,
            textAlign: "center",
          }}
        >
          <div style={{ fontSize: 18, fontWeight: 600 }}>Agency</div>
          <div style={{ fontSize: 13, color: "#f38ba8", maxWidth: 560, wordBreak: "break-word" }}>
            {this.state.error.message || String(this.state.error)}
          </div>
          <button
            onClick={() => location.reload()}
            style={{
              marginTop: 8,
              padding: "6px 16px",
              borderRadius: 6,
              border: "1px solid #45475a",
              background: "#313244",
              color: "#cdd6f4",
              fontSize: 13,
              cursor: "pointer",
            }}
          >
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <RootErrorBoundary>
      <App />
    </RootErrorBoundary>
  </React.StrictMode>,
);
