import { MenuContext } from "../lib/appMenu";
import { IS_LINUX, IS_TAURI, shortcutLabel } from "../lib/platform";
import AppMenuBar from "./AppMenuBar";
import WindowControls from "./WindowControls";

const NO_CONTEXT: MenuContext = { project: false, focusedAgent: false, gitless: false };

export default function TitleBar({
  onOpenPalette,
  // First run has nothing to search yet, so the bar is only a drag handle there
  // (and, on Linux, the window's only controls).
  bare = false,
  menuContext = NO_CONTEXT,
  onMenu,
}: {
  onOpenPalette: () => void;
  bare?: boolean;
  menuContext?: MenuContext;
  /** Routes a menu action; only the frontend-drawn menu (Linux) calls it. */
  onMenu?: (action: string) => void;
}) {
  return (
    // data-tauri-drag-region="deep" makes the whole bar a drag handle. Tauri
    // auto-excludes clickable elements (button/input/etc), so the controls below
    // stay interactive without any extra markup. A double-click on the handle
    // maximizes, which is what an undecorated Linux window's title row owes.
    <header className="titlebar" data-tauri-drag-region="deep">
      <div className="titlebar-left">
        {IS_LINUX && IS_TAURI && !bare && onMenu && <AppMenuBar context={menuContext} onAction={onMenu} />}
      </div>
      {!bare && (
        <button className="search-box" onClick={onOpenPalette}>
          <span>⌕</span>
          {/* Keep this in sync with what CommandPalette actually searches —
              files/issues/contents join in Phase 4 (palette v2). */}
          <span className="search-ph">Search projects and agents…</span>
          <span className="kbd">{shortcutLabel("⌘K")}</span>
        </button>
      )}
      <div className="titlebar-right">
        {IS_LINUX && IS_TAURI && <WindowControls />}
      </div>
    </header>
  );
}
