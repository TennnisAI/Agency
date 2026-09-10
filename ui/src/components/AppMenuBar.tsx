import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { requestQuit } from "../api";
import { APP_MENUS, MenuContext, MenuItemSpec, itemEnabled, itemForKey } from "../lib/appMenu";
import { shortcutLabel } from "../lib/platform";
import Menu, { MenuEntry } from "./git/Menu";

/**
 * The application menu drawn in the title bar, for the platforms with no
 * native one (Linux). Titles open on click and switch on hover while one is
 * open, like a menu bar. Every item routes through `onAction` with the same
 * `menu:` names the native menu emits, so App.onMenu is the one switch for
 * both; window and editing items are handled here, since they never reached
 * the backend on macOS either.
 *
 * The chords work with no native menu to own them: the binder below fires
 * the ones the app does not already bind (see appMenu.ts `boundElsewhere`).
 */
export default function AppMenuBar({
  context,
  onAction,
}: {
  context: MenuContext;
  onAction: (action: string) => void;
}) {
  const [open, setOpen] = useState<number | null>(null);
  const [anchor, setAnchor] = useState<{ x: number; y: number } | null>(null);
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  function dispatch(action: string) {
    if (action.startsWith("win:")) {
      const win = getCurrentWindow();
      switch (action) {
        case "win:minimize": void win.minimize(); break;
        case "win:toggle-maximize": void win.toggleMaximize(); break;
        case "win:close": void win.close(); break;
        case "win:fullscreen":
          win.isFullscreen().then((f) => win.setFullscreen(!f)).catch(() => {});
          break;
      }
      return;
    }
    if (action.startsWith("edit:")) {
      // The webview's own editing commands, aimed at whatever has focus.
      document.execCommand(action.slice("edit:".length));
      return;
    }
    if (action === "quit") {
      requestQuit().catch(() => {});
      return;
    }
    onAction(action);
  }

  const contextRef = useRef(context);
  contextRef.current = context;
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const item = itemForKey(e);
      if (!item) return;
      e.preventDefault();
      if (itemEnabled(item, contextRef.current)) dispatch(item.action);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function show(i: number) {
    const b = buttons.current[i];
    if (!b) return;
    const r = b.getBoundingClientRect();
    setAnchor({ x: r.left, y: r.bottom + 2 });
    setOpen(i);
  }

  function entries(items: typeof APP_MENUS[number]["items"]): MenuEntry[] {
    return items.map((it) =>
      it.kind === "separator"
        ? { kind: "separator" }
        : {
            label: it.label,
            hint: it.accel ? shortcutLabel(it.accel) : undefined,
            disabled: !itemEnabled(it as MenuItemSpec, context),
            onClick: () => dispatch(it.action),
          },
    );
  }

  return (
    <nav className={`menubar${open !== null ? " open" : ""}`} aria-label="Application menu">
      {APP_MENUS.map((m, i) => (
        <button
          key={m.title}
          type="button"
          ref={(el) => { buttons.current[i] = el; }}
          className={`menubar-title${open === i ? " open" : ""}`}
          onMouseDown={(e) => {
            e.preventDefault();
            e.stopPropagation();
            if (open === i) setOpen(null);
            else show(i);
          }}
          onMouseEnter={() => { if (open !== null && open !== i) show(i); }}
        >
          {m.title}
        </button>
      ))}
      {open !== null && anchor && (
        <Menu x={anchor.x} y={anchor.y} items={entries(APP_MENUS[open].items)} onClose={() => setOpen(null)} />
      )}
    </nav>
  );
}
