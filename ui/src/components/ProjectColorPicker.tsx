import { useEffect } from "react";
import { PROJECT_COLOR_NAMES } from "../agents";

// Swatch popover for a project's sidebar icon, opened by double-clicking the
// icon. The choices are the theme's accent palette rather than a free-form
// color well on purpose: a project color is stored as an accent name and
// resolved through `var(--<name>)`, so it keeps following the active theme.
export default function ProjectColorPicker({
  anchor,
  current,
  onPick,
  onClose,
}: {
  /** Viewport rect of the icon that opened this, so the popover hangs off it. */
  anchor: DOMRect;
  current: string | null;
  onPick: (color: string) => void;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Fixed coordinates, like the agent menu: an ancestor's `overflow: hidden`
  // (the scrolling tree) would otherwise clip it. Nudged back inside the
  // viewport when the icon sits near the bottom.
  const WIDTH = 132;
  const HEIGHT = 108;
  const top = Math.min(anchor.bottom + 6, Math.max(8, window.innerHeight - HEIGHT - 8));
  const left = Math.min(anchor.left, Math.max(8, window.innerWidth - WIDTH - 8));

  return (
    <>
      <div className="agent-menu-backdrop" onClick={onClose} />
      <div
        className="color-menu"
        style={{ position: "fixed", top, left }}
        role="dialog"
        aria-label="Project color"
        onClick={(e) => e.stopPropagation()}
      >
        {PROJECT_COLOR_NAMES.map((c) => (
          <button
            key={c}
            className={`color-swatch${c === current ? " on" : ""}`}
            style={{ background: `var(--${c})` }}
            title={c}
            aria-label={c}
            aria-pressed={c === current}
            onClick={() => onPick(c)}
          />
        ))}
      </div>
    </>
  );
}
