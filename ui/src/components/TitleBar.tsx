export default function TitleBar({
  onOpenPalette,
}: {
  onOpenPalette: () => void;
}) {
  return (
    // data-tauri-drag-region="deep" makes the whole bar a drag handle. Tauri
    // auto-excludes clickable elements (button/input/etc), so the controls below
    // stay interactive without any extra markup.
    <header className="titlebar" data-tauri-drag-region="deep">
      <div className="titlebar-left" />
      <button className="search-box" onClick={onOpenPalette}>
        <span>⌕</span>
        <span className="search-ph">Search projects, tasks, files…</span>
        <span className="kbd">⌘K</span>
      </button>
      <div className="titlebar-right" />
    </header>
  );
}
