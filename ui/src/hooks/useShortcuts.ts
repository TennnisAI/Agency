import { useEffect } from "react";

interface Handlers {
  onNewTask: () => void;
  onSource: () => void;
  onApprove: () => void;
  onPalette: () => void;
}

function inEditable(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
}

export function useShortcuts(h: Handlers): void {
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "k") {
        e.preventDefault();
        h.onPalette();
      } else if (key === "enter") {
        e.preventDefault();
        h.onApprove();
      } else if (key === "n" && !inEditable(e.target)) {
        e.preventDefault();
        h.onNewTask();
      } else if (key === "g" && !inEditable(e.target)) {
        e.preventDefault();
        h.onSource();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [h]);
}
