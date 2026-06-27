export default function TitleBar({
  sidebarOpen,
  onToggleSidebar,
  onOpenSettings,
  onOpenPalette,
}: {
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onOpenSettings: () => void;
  onOpenPalette: () => void;
}) {
  return (
    // data-tauri-drag-region="deep" makes the whole bar a drag handle. Tauri
    // auto-excludes clickable elements (button/input/etc), so the controls below
    // stay interactive without any extra markup.
    <header className="titlebar" data-tauri-drag-region="deep">
      <div className="titlebar-left">
        <button
          className="icon-btn"
          title={sidebarOpen ? "Hide sidebar" : "Show sidebar"}
          onClick={onToggleSidebar}
        >
          <SidebarIcon filled={!sidebarOpen} />
        </button>
      </div>
      <button className="search-box" onClick={onOpenPalette}>
        <span>⌕</span>
        <span className="search-ph">Search projects, tasks, files…</span>
        <span className="kbd">⌘K</span>
      </button>
      <div className="titlebar-right">
        <button className="icon-btn" title="Settings" onClick={onOpenSettings}>
          ⚙
        </button>
      </div>
    </header>
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
