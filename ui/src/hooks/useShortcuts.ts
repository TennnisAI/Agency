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
  if (tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable) return true;
  // xterm renders into non-INPUT/TEXTAREA elements; also treat contenteditable
  // and role=textbox targets as editable so shortcuts don't fire inside them.
  return !!el.closest?.(".xterm, [contenteditable], [role=\"textbox\"]");
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
      // Skip when typing into an editable/terminal target. The focused-agent
      // requirement for approval is enforced by the caller's onApprove.
      else if (key === "enter" && !inEditable(e.target)) { e.preventDefault(); ref.current.onApprove(); }
      else if (key === "n" && !inEditable(e.target)) { e.preventDefault(); ref.current.onNewTask(); }
      else if (key === "g" && !inEditable(e.target)) { e.preventDefault(); ref.current.onSource(); }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []); // bind once
}
