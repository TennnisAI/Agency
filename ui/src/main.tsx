import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, getStoredTheme } from "./lib/themes";
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

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
