import { useEffect, useRef } from "react";

interface Handlers {
  onNewTask: () => void;
  onSource: () => void;
  onApprove: () => void;
  onPalette: () => void;
  onSettings: () => void;
}

function inEditable(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
}

export function useShortcuts(h: Handlers): void {
  const ref = useRef(h);
  ref.current = h; // keep latest handlers without re-binding the listener
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "k") { e.preventDefault(); ref.current.onPalette(); }
      else if (key === ",") { e.preventDefault(); ref.current.onSettings(); }
      else if (key === "enter") { e.preventDefault(); ref.current.onApprove(); }
      else if (key === "n" && !inEditable(e.target)) { e.preventDefault(); ref.current.onNewTask(); }
      else if (key === "g" && !inEditable(e.target)) { e.preventDefault(); ref.current.onSource(); }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []); // bind once
}
