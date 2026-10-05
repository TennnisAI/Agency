import { focusPopout } from "../api";
import { bringBack } from "../hooks/usePopouts";
import { PopoutEntry } from "../lib/popout";

/**
 * What the main window shows where a popped-out item would be (AGE-252). The
 * item itself stays in its own window, so this pane never mounts a second
 * terminal or a second editor on it; it offers the way to that window and the
 * way back from it.
 */
export default function PoppedOut({
  entry,
  what,
  compact = false,
}: {
  entry: PopoutEntry;
  /** What is out, as the sentence names it: "This agent", "notes.md". */
  what: string;
  /** A narrow pane (the agents side panel): drop the explanation line. */
  compact?: boolean;
}) {
  return (
    <div className={`popped-out${compact ? " compact" : ""}`}>
      <div className="popped-out-glyph" aria-hidden><PopOutGlyph /></div>
      <div className="popped-out-title">{what} is open in its own window.</div>
      {!compact && (
        <div className="popped-out-sub">Bring it back to show it here again.</div>
      )}
      <div className="popped-out-actions">
        <button className="btn-secondary" onClick={() => { focusPopout(entry.label).catch(() => {}); }}>
          Show window
        </button>
        <button className="btn-primary" onClick={() => bringBack(entry.label)}>
          Bring back here
        </button>
      </div>
    </div>
  );
}

const g = {
  width: 15, height: 15, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor",
  strokeWidth: 1.9, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, "aria-hidden": true,
};

/** A box with an arrow leaving it: open in a new window. */
export function PopOutGlyph() {
  return (
    <svg {...g}>
      <path d="M14 4h6v6" />
      <path d="M20 4l-9 9" />
      <path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4" />
    </svg>
  );
}

/** An arrow coming into a box: put this back in the main window. */
export function ReattachGlyph() {
  return (
    <svg {...g}>
      <path d="M11 13V7" />
      <path d="M11 13H5" />
      <path d="M11 13L3 5" />
      <path d="M14 4h4a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-4" />
    </svg>
  );
}
