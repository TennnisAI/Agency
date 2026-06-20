export default function TitleBar({
  onToggleSidebar,
  onOpenSettings,
}: {
  onToggleSidebar: () => void;
  onOpenSettings: () => void;
}) {
  return (
    <header className="titlebar">
      <div className="titlebar-left">
        <button className="icon-btn no-drag" title="Toggle sidebar" onClick={onToggleSidebar}>
          ☰
        </button>
        <span className="logo-mark" />
        <span className="app-name">Agency</span>
      </div>
      <div className="search-box no-drag" aria-disabled>
        <span>⌕</span>
        <span className="search-ph">Search projects, tasks, files…</span>
        <span className="kbd">⌘K</span>
      </div>
      <div className="titlebar-right">
        <button className="icon-btn no-drag" title="Settings" onClick={onOpenSettings}>
          ⚙
        </button>
      </div>
    </header>
  );
}
