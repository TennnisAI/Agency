import { useEffect, useState } from "react";

// Shared keyboard navigation for palette-style lists (command palette, quick
// switchers): ArrowUp/Down move the highlight, Enter activates it. Escape is
// deliberately not handled — that's useModalKeys' job. `resetKey` snaps the
// highlight back to the top when it changes (typically the query string).
export function useListNav(
  count: number,
  activate: (index: number) => void,
  resetKey?: unknown,
): { hi: number; setHi: (i: number) => void; onKey: (ev: React.KeyboardEvent) => void } {
  const [hi, setHi] = useState(0);
  useEffect(() => {
    setHi(0);
  }, [resetKey]);

  // Clamp instead of resetting when the list shrinks under the highlight
  // (async results replacing a longer list).
  const cur = Math.min(hi, Math.max(0, count - 1));

  function onKey(ev: React.KeyboardEvent) {
    if (ev.key === "ArrowDown") {
      ev.preventDefault();
      setHi(count === 0 ? 0 : Math.min(cur + 1, count - 1));
    } else if (ev.key === "ArrowUp") {
      ev.preventDefault();
      setHi(Math.max(cur - 1, 0));
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      if (count > 0) activate(cur);
    }
  }

  return { hi: cur, setHi, onKey };
}
