// Sidebar show/hide control. Lives in the Projects pane header while the
// sidebar is open; when hidden it re-appears in the content header, to the
// left of the Agents / Source Control / Files selector.
export default function SidebarToggle({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      className="icon-btn"
      title={open ? "Hide sidebar" : "Show sidebar"}
      onClick={onToggle}
    >
      <SidebarIcon filled={!open} />
    </button>
  );
}

// Standard sidebar-toggle glyph: a panel with a left rail. Outline when the
// sidebar is showing (click to hide); the rail is filled in when it's hidden
// (click to show).
function SidebarIcon({ filled }: { filled: boolean }) {
  return (
    <svg
      width="17"
      height="17"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect x="3" y="3" width="18" height="18" rx="2" />
      {filled ? (
        <path d="M9 4 H6 a2 2 0 0 0 -2 2 V18 a2 2 0 0 0 2 2 H9 Z" fill="currentColor" stroke="none" />
      ) : (
        <line x1="9" y1="3" x2="9" y2="21" />
      )}
    </svg>
  );
}
