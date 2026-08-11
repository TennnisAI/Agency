import type { ILink, Terminal } from "@xterm/xterm";
import { FileRoot, LinkedPath, resolveTermPaths } from "../api";
import { linkRange, logicalLine, scanLine, TermLinkCandidate } from "./termLinks";

// The xterm half of clickable terminal links (AGE-94): it asks lib/termLinks
// what the hovered line holds, checks the path-shaped answers against disk, and
// hands what survives to the pane's own openers.
//
// The gesture is ⌘-click (Ctrl elsewhere), the same one the app's wikilinks and
// every macOS terminal use, and links only underline while the modifier is
// held. That is not decoration: an agent pane runs a mouse-tracking TUI, so a
// plain click belongs to the program underneath, and only the modified one is
// unambiguously meant for the link.

/** How long "nothing there" is trusted before a hover asks disk again. */
const MISS_TTL_MS = 20_000;
/** Cache ceiling — a long-lived pane hovers a lot of lines. */
const CACHE_MAX = 1000;
/** Matches the backend's per-call cap (`MAX_LINK_CANDIDATES`). */
const MAX_PER_LINE = 32;

export interface TermLinkHandlers {
  /** The root paths resolve against; null when no project is selected. */
  root: () => FileRoot | null;
  /** http(s)/mailto, straight to the browser. */
  openUrl: (url: string) => void;
  /**
   * A filesystem path. `hit` is the resolution the hover already fetched, or
   * null when the click came from an OSC 8 `file://` link and nothing has
   * resolved it yet.
   */
  openPath: (path: string, line: number | undefined, hit: LinkedPath | null) => void;
}

const isModified = (e: MouseEvent | KeyboardEvent) => e.metaKey || e.ctrlKey;

/** Schemes a terminal is allowed to launch. Program output isn't trusted with more. */
const WEB_SCHEME = /^(https?:|mailto:)/i;

/**
 * Wire link detection into a terminal. Returns the teardown; the terminal's own
 * dispose would cover the xterm side, but the window and element listeners are
 * ours to remove.
 */
export function installTermLinks(
  term: Terminal,
  container: HTMLElement,
  handlers: TermLinkHandlers,
): () => void {
  let torn = false;
  let held = false;
  // The link under the pointer, if any. A set rather than a field because
  // hover/leave are the only truth we get and they can interleave.
  const hovered = new Set<ILink>();
  // An OSC 8 link (one the program marked up itself) has no ILink we own, so
  // its hover is tracked separately — the click swallow needs to know.
  let oscHovered = false;
  const resolved = new Map<string, { hit: LinkedPath | null; at: number }>();

  const usable = (entry: { hit: LinkedPath | null; at: number }) =>
    entry.hit !== null || Date.now() - entry.at < MISS_TTL_MS;

  const remember = (path: string, hit: LinkedPath | null) => {
    if (resolved.size >= CACHE_MAX) resolved.clear();
    resolved.set(path, { hit, at: Date.now() });
  };

  // xterm tracks writes to a hovered link's `decorations`, so the underline can
  // follow the modifier while the pointer sits still. The object it tracks is
  // installed just after our `hover` callback returns, hence the microtask.
  const decorate = () => {
    for (const link of hovered) {
      if (!link.decorations) continue;
      link.decorations.underline = held;
      link.decorations.pointerCursor = held;
    }
  };
  const setHeld = (next: boolean) => {
    if (next === held) return;
    held = next;
    decorate();
  };

  const follow = (uri: string) => {
    if (WEB_SCHEME.test(uri)) {
      handlers.openUrl(uri);
      return;
    }
    // file:// — an OSC 8 link to something on disk, which the pane opens the
    // same way it opens a path it found itself.
    if (/^file:/i.test(uri)) {
      try {
        handlers.openPath(decodeURIComponent(new URL(uri).pathname), undefined, null);
      } catch {
        /* not a URL we can make sense of */
      }
    }
    // Any other scheme is program output asking the OS to launch something —
    // not a link this terminal offers.
  };

  const makeLink = (
    candidate: TermLinkCandidate,
    firstRow: number,
    activate: () => void,
  ): ILink => {
    const link: ILink = {
      range: linkRange(term.cols, firstRow, candidate.start, candidate.end),
      text: candidate.target,
      decorations: { underline: held, pointerCursor: held },
      activate: (e) => {
        if (!isModified(e)) return; // a plain click is the program's
        term.focus(); // the swallow below took xterm's own focus-on-click away
        activate();
      },
      hover: (e) => {
        hovered.add(link);
        setHeld(isModified(e));
        queueMicrotask(decorate);
      },
      leave: () => {
        hovered.delete(link);
      },
    };
    return link;
  };

  const provider = term.registerLinkProvider({
    provideLinks(y, callback) {
      const line = logicalLine(term.buffer.active, y);
      if (!line) return callback(undefined);
      const found = scanLine(line.text);
      if (!found.length) return callback(undefined);

      const deliver = () => {
        if (torn) return;
        const links: ILink[] = [];
        for (const candidate of found) {
          if (candidate.kind === "url") {
            links.push(makeLink(candidate, line.firstRow, () => follow(candidate.target)));
            continue;
          }
          const entry = resolved.get(candidate.target);
          if (!entry?.hit) continue; // nothing on disk — not a link
          const hit = entry.hit;
          links.push(
            makeLink(candidate, line.firstRow, () =>
              handlers.openPath(candidate.target, candidate.line, hit)),
          );
        }
        callback(links.length ? links : undefined);
      };

      const root = handlers.root();
      const paths = [...new Set(found.filter((c) => c.kind === "path").map((c) => c.target))]
        .filter((p) => {
          const entry = resolved.get(p);
          return !entry || !usable(entry);
        })
        .slice(0, MAX_PER_LINE);
      // Nothing to check (or nowhere to check it): URLs alone still linkify.
      if (!paths.length || !root) return deliver();
      resolveTermPaths(root, paths)
        .then((hits) => paths.forEach((p, i) => remember(p, hits[i] ?? null)))
        .catch(() => paths.forEach((p) => remember(p, null)))
        .then(deliver);
    },
  });

  // OSC 8 hyperlinks: text the program itself marked up. xterm decorates those
  // on plain hover (their decorations aren't ours to gate), but following one
  // still takes the modifier, so one gesture opens every kind of link.
  term.options.linkHandler = {
    allowNonHttpProtocols: true, // `follow` is the filter, so file:// survives
    activate: (e, uri) => {
      if (!isModified(e)) return;
      term.focus();
      follow(uri);
    },
    hover: () => { oscHovered = true; },
    leave: () => { oscHovered = false; },
  };

  const onKey = (e: KeyboardEvent) => setHeld(isModified(e));
  const onBlur = () => setHeld(false);
  window.addEventListener("keydown", onKey);
  window.addEventListener("keyup", onKey);
  window.addEventListener("blur", onBlur);

  // A ⌘-click on a link is meant for the link, not for the program in the pane.
  // xterm reports mouse events from a listener on the terminal element, one
  // level above the screen this listens on, so stopping propagation here keeps
  // the agent's TUI from also being clicked — while xterm's own link handling,
  // which listens on this same element, still runs.
  const screen = container.querySelector<HTMLElement>(".xterm-screen");
  const swallow = (e: MouseEvent) => {
    if (!(hovered.size || oscHovered) || !isModified(e)) return;
    e.preventDefault();
    e.stopPropagation();
  };
  screen?.addEventListener("mousedown", swallow);
  screen?.addEventListener("mouseup", swallow);

  return () => {
    torn = true;
    window.removeEventListener("keydown", onKey);
    window.removeEventListener("keyup", onKey);
    window.removeEventListener("blur", onBlur);
    screen?.removeEventListener("mousedown", swallow);
    screen?.removeEventListener("mouseup", swallow);
    provider.dispose();
  };
}
