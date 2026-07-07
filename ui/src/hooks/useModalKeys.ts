import { useEffect, useRef } from "react";
import { pushModal, topModal } from "../lib/modalStack";

// Escape-to-close for modals. Registers on the shared modal stack so only the
// topmost dialog closes per keypress. Pass `enabled: false` while the modal is
// busy (mirrors a disabled Cancel button). Escape typed into a terminal is
// left alone — xterm owns keys there.
export function useModalKeys(onClose: () => void, enabled = true): void {
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    if (!enabled) return;
    const handler = () => closeRef.current();
    const unregister = pushModal(handler);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if ((e.target as HTMLElement | null)?.closest?.(".terminal, .xterm")) return;
      if (topModal() !== handler) return;
      e.preventDefault();
      e.stopPropagation();
      handler();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      unregister();
      window.removeEventListener("keydown", onKey);
    };
  }, [enabled]);
}
