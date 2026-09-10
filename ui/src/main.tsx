import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, getStoredTheme } from "./lib/themes";
import { toastError } from "./lib/toast";
import { PLATFORM } from "./lib/platform";
import "./theme.css";
import "./styles.css";

applyTheme(getStoredTheme());
// The stylesheet reads this for the one thing that differs per desktop: the
// title bar's inset for the macOS traffic lights, and its own controls on Linux.
document.documentElement.dataset.platform = PLATFORM;

// Suppress the webview's default right-click menu (Reload / Developer Tools),
// which is irrelevant to end users. Terminals and editable text fields keep
// their native menu so copy/paste still works there.
window.addEventListener("contextmenu", (e) => {
  const target = e.target as HTMLElement | null;
  if (target?.closest("input, textarea, [contenteditable=\"true\"], .terminal, .xterm")) return;
  e.preventDefault();
});

// Benign browser noise that must never reach the user as an error. Both of
// these escape every component handler by construction — they are dispatched at
// the window, from work the component no longer has a handle on — so this is
// the only place they can be filtered.
//
// 1. WebKit reports "ResizeObserver loop completed with undelivered
//    notifications" whenever an observer callback resizes its own subject and
//    the work spills past the frame, which is what a terminal fitting itself
//    inside a pane that is still settling does. Nothing is broken and nothing
//    is dropped: the browser delivers the rest next frame.
//
// 2. xterm reads its render service's dimensions off a renderer that has
//    already been disposed. FocusTerminal's teardown waits for the write buffer
//    to drain and for a frame to pass before disposing, which is what xterm
//    gives us to wait on (see the AGE-124 comment there) — but xterm 5.5.0
//    schedules work on bare timers and animation frames it neither cancels nor
//    guards, so a window that stops animating mid-teardown can still land one
//    of those reads after the dispose. What it would be reading for is a paint
//    of a pane that is already unmounted, so there is nothing to tell the user
//    and nothing they could do; a live pane failing to render shows as a blank
//    pane, not as this.
const isBenign = (v: unknown) => {
  const text = typeof v === "string" ? v : v instanceof Error ? v.message : "";
  return text.startsWith("ResizeObserver loop") || text.includes("_renderer.value.dimensions");
};

// Surface errors that escape every component handler. The toast bus dedupes
// identical messages, so a rejecting poll loop can't stack toasts.
window.addEventListener("error", (e) => {
  if (isBenign(e.error ?? e.message)) return;
  toastError(e.error ?? e.message, "Unexpected error");
});
window.addEventListener("unhandledrejection", (e) => {
  if (isBenign(e.reason)) return;
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
