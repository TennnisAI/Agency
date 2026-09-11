// The application menu, as data, for the platforms where Agency draws it
// itself.
//
// On macOS the menu bar is native (crates/agency-app/src/menu.rs) and this
// file is not consulted. On Linux the window is undecorated and the title bar
// carries these menus, so the bar the window manager would have drawn, the GTK
// menu strip and Agency's own title row collapse into one (observed
// 2026-09-10: three bars, 110px, before any content). Keep the two in step:
// every `menu:` action here is one `App.onMenu` already routes, and the
// labels and chords are the native menu's.
//
// Chords are spelled the Mac way and translated by `shortcutLabel`. The
// keydown binder below is what makes them work with no native menu to own
// them; items the app binds elsewhere are marked so they are not fired twice.

export type MenuContext = {
  project: boolean;
  focusedAgent: boolean;
  gitless: boolean;
};

export type MenuItemSpec = {
  kind?: "item";
  label: string;
  /** A `menu:` action for App.onMenu, `win:` for the window, `edit:` for the webview's own editing commands. */
  action: string;
  /** Mac-spelled chord, e.g. "⌘⇧D". */
  accel?: string;
  /** What has to be true for the item to do anything. */
  needs?: keyof MenuContext;
  /**
   * The chord is already bound elsewhere in the app (useShortcuts, App's
   * find handler, or the webview itself for editing), so the binder must not
   * fire it a second time. The label still shows the chord.
   */
  boundElsewhere?: boolean;
};

export type MenuEntrySpec = MenuItemSpec | { kind: "separator" };

export type MenuSpec = { title: string; items: MenuEntrySpec[] };

const sep: MenuEntrySpec = { kind: "separator" };

export const APP_MENUS: MenuSpec[] = [
  {
    title: "Agency",
    items: [
      { label: "Settings…", action: "settings", accel: "⌘,", boundElsewhere: true },
      sep,
      { label: "Quit Agency", action: "quit", accel: "⌘Q" },
    ],
  },
  {
    title: "File",
    items: [
      { label: "New Agent", action: "new-agent", accel: "⌘N", needs: "project", boundElsewhere: true },
      { label: "New Terminal", action: "new-terminal", accel: "⌘T", needs: "project" },
      sep,
      { label: "Today's Note", action: "daily-note", accel: "⌘⇧D", boundElsewhere: true },
      { label: "Generate Weekly Note", action: "weekly-note" },
      sep,
      { label: "Add Project…", action: "add-project", accel: "⌘⇧O" },
      { label: "Clone Repository…", action: "clone-project" },
      { label: "Initialize Git Repository…", action: "init-repo", needs: "gitless" },
      sep,
      // No accelerator: Ctrl+W closes a file or docs tab in the app, and the
      // label promised a chord nothing bound to the window.
      { label: "Close Window", action: "win:close" },
    ],
  },
  {
    title: "Edit",
    items: [
      { label: "Undo", action: "edit:undo", accel: "⌘Z", boundElsewhere: true },
      { label: "Redo", action: "edit:redo", accel: "⌘⇧Z", boundElsewhere: true },
      sep,
      { label: "Cut", action: "edit:cut", accel: "⌘X", boundElsewhere: true },
      { label: "Copy", action: "edit:copy", accel: "⌘C", boundElsewhere: true },
      { label: "Paste", action: "edit:paste", accel: "⌘V", boundElsewhere: true },
      { label: "Select All", action: "edit:selectAll", accel: "⌘A", boundElsewhere: true },
      sep,
      { label: "Find…", action: "find", accel: "⌘F", boundElsewhere: true },
      { label: "Find and Replace…", action: "replace", accel: "⌥⌘F" },
      { label: "Find Next", action: "find-next", accel: "⌘G" },
      { label: "Find Previous", action: "find-prev", accel: "⌘⇧G" },
    ],
  },
  {
    title: "View",
    items: [
      { label: "Command Palette…", action: "palette", accel: "⌘K", boundElsewhere: true },
      { label: "Source & Diff", action: "source", accel: "⌘D", needs: "project", boundElsewhere: true },
      sep,
      { label: "Toggle Sidebar", action: "toggle-sidebar", accel: "⌘B" },
      { label: "All Projects", action: "home", accel: "⌘⇧H" },
      sep,
      { label: "Toggle Full Screen", action: "win:fullscreen", accel: "F11" },
    ],
  },
  {
    title: "Agent",
    items: [
      { label: "Approve & Merge", action: "approve", accel: "⌘↵", needs: "focusedAgent", boundElsewhere: true },
      sep,
      { label: "Archive Agent", action: "archive", needs: "focusedAgent" },
      { label: "Delete Agent", action: "discard", needs: "focusedAgent" },
    ],
  },
  {
    title: "Window",
    items: [
      { label: "Minimize", action: "win:minimize" },
      { label: "Maximize", action: "win:toggle-maximize" },
      sep,
      { label: "Close Window", action: "win:close" },
    ],
  },
  {
    title: "Help",
    items: [
      { label: "Report an Issue…", action: "report-issue" },
      { label: "Agency on GitHub", action: "github" },
    ],
  },
];

export function itemEnabled(item: MenuItemSpec, ctx: MenuContext): boolean {
  return item.needs ? ctx[item.needs] : true;
}

/** The subset of a keyboard event the chord matcher reads. */
export type KeyLike = {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
};

/**
 * Whether `e` is the chord `accel` spells. ⌘ is the platform command key,
 * which on Linux is Ctrl (and on a Mac keyboard plugged into one, still
 * Ctrl: `metaKey` there is the Super key, which no chord uses, so a chord
 * with Super held is not the chord).
 */
export function chordMatches(e: KeyLike, accel: string): boolean {
  let cmd = false;
  let shift = false;
  let alt = false;
  let key = "";
  for (const ch of accel) {
    if (ch === "⌘" || ch === "⌃") cmd = true;
    else if (ch === "⇧") shift = true;
    else if (ch === "⌥") alt = true;
    else key += ch;
  }
  const want = key === "↵" ? "enter" : key.toLowerCase();
  const pressed = e.key.toLowerCase();
  return (
    pressed === want &&
    e.ctrlKey === cmd &&
    !e.metaKey &&
    e.shiftKey === shift &&
    e.altKey === alt
  );
}

/**
 * The item `e` fires, if any, among the ones the binder owns (chorded, and not
 * bound elsewhere). Disabled items still match, so the key is swallowed rather
 * than reaching the webview as something else; the caller checks `itemEnabled`
 * before acting.
 */
export function itemForKey(e: KeyLike, menus: MenuSpec[] = APP_MENUS): MenuItemSpec | null {
  for (const menu of menus) {
    for (const entry of menu.items) {
      if (entry.kind === "separator" || !entry.accel || entry.boundElsewhere) continue;
      if (chordMatches(e, entry.accel)) return entry;
    }
  }
  return null;
}
