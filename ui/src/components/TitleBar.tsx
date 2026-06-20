export default function TitleBar({
  onToggleSidebar,
  onOpenSettings,
  onOpenPalette,
}: {
  onToggleSidebar: () => void;
  onOpenSettings: () => void;
  onOpenPalette: () => void;
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
      <button className="search-box no-drag" onClick={onOpenPalette}>
        <span>⌕</span>
        <span className="search-ph">Search projects, tasks, files…</span>
        <span className="kbd">⌘K</span>
      </button>
      <div className="titlebar-right">
        <button className="icon-btn no-drag" title="Settings" onClick={onOpenSettings}>
          ⚙
        </button>
      </div>
    </header>
  );
}
