import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Minimize, maximize and close, for the platforms where the window is
 * undecorated and Agency's title bar is the only one (Linux; see
 * tauri.linux.conf.json). Close goes through the window's close request,
 * which the backend turns into a hide, the same as the native button did.
 * Glyphs, not icons: the app's rule is monochrome text everywhere.
 */
export default function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    let live = true;
    const read = () => {
      win.isMaximized().then((m) => { if (live) setMaximized(m); }).catch(() => {});
    };
    read();
    const un = win.onResized(read);
    return () => {
      live = false;
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  const win = () => getCurrentWindow();
  return (
    <div className="win-controls" role="group" aria-label="Window">
      <button type="button" className="win-btn" title="Minimize" aria-label="Minimize" onClick={() => void win().minimize()}>
        <span aria-hidden>─</span>
      </button>
      <button type="button" className="win-btn" title={maximized ? "Restore" : "Maximize"} aria-label={maximized ? "Restore" : "Maximize"} onClick={() => void win().toggleMaximize()}>
        <span aria-hidden>{maximized ? "❐" : "□"}</span>
      </button>
      <button type="button" className="win-btn close" title="Close" aria-label="Close" onClick={() => void win().close()}>
        <span aria-hidden>✕</span>
      </button>
    </div>
  );
}
