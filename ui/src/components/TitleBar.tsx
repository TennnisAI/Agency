export default function TitleBar({
  onOpenPalette,
  // First run has nothing to search yet, so the bar is only a drag handle there.
  bare = false,
}: {
  onOpenPalette: () => void;
  bare?: boolean;
}) {
  return (
    // data-tauri-drag-region="deep" makes the whole bar a drag handle. Tauri
    // auto-excludes clickable elements (button/input/etc), so the controls below
    // stay interactive without any extra markup.
    <header className="titlebar" data-tauri-drag-region="deep">
      <div className="titlebar-left" />
      {!bare && (
        <button className="search-box" onClick={onOpenPalette}>
          <span>⌕</span>
          {/* Keep this in sync with what CommandPalette actually searches —
              files/issues/contents join in Phase 4 (palette v2). */}
          <span className="search-ph">Search projects and agents…</span>
          <span className="kbd">⌘K</span>
        </button>
      )}
      <div className="titlebar-right" />
    </header>
  );
}
