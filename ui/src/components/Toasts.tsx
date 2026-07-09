import { useEffect, useState } from "react";
import { ToastMsg, dismissToast, subscribeToasts } from "../lib/toast";

const GLYPH: Record<ToastMsg["kind"], string> = { success: "✓", error: "✕", info: "◦" };

/** Toast stack (errors, success, info), mounted once at the shell root. */
export default function Toasts() {
  const [toasts, setToasts] = useState<ToastMsg[]>([]);
  useEffect(() => subscribeToasts(setToasts), []);
  if (toasts.length === 0) return null;
  return (
    <div className="toast-stack">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast-${t.kind}`} onClick={() => dismissToast(t.id)}>
          <span className="toast-glyph" aria-hidden>{GLYPH[t.kind]}</span>
          <span className="toast-text">{t.text}</span>
          <button className="toast-close" onClick={() => dismissToast(t.id)}>✕</button>
        </div>
      ))}
    </div>
  );
}
