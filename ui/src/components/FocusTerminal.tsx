import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import "@xterm/xterm/css/xterm.css";
import { attachRun, detachRun, resizeRun, runInput, runPreview, ensureRunActive,
  attachRunScript, detachRunScript, resizeRunScript, runScriptInput, runScriptPreview,
  attachShell, detachShell, resizeShell, shellInput, shellPreview, startShell } from "../api";
import { currentXtermTheme, minContrastRatio, TERMINAL_FONT_FAMILY } from "../lib/themes";
import { initialCapture, feed } from "../lib/firstPrompt";

export interface TerminalStream {
  attach(id: string, onBytes: (b: Uint8Array) => void): Promise<void>;
  detach(id: string): void;
  resize(id: string, cols: number, rows: number): Promise<void>;
  input(id: string, data: string): Promise<void>;
  preview(id: string, lines: number): Promise<string>;
  ensureActive?: (id: string) => Promise<void>;
}

export const agentStream: TerminalStream = {
  attach: attachRun, detach: detachRun, resize: resizeRun, input: runInput, preview: runPreview,
  ensureActive: ensureRunActive,
};

export const runStream: TerminalStream = {
  attach: attachRunScript, detach: detachRunScript, resize: resizeRunScript,
  input: runScriptInput, preview: runScriptPreview,
};

// Companion shell. `ensureActive` lazily starts (or reuses) the worktree shell
// so the panel can be opened without an explicit "start" step; start_shell is
// idempotent, so remounting the panel resumes the same session.
export const shellStream: TerminalStream = {
  attach: attachShell, detach: detachShell, resize: resizeShell,
  input: shellInput, preview: shellPreview, ensureActive: startShell,
};

export default function FocusTerminal(
  { runId, stream = agentStream, onFirstPrompt }: { runId: string; stream?: TerminalStream; onFirstPrompt?: (line: string) => void },
) {
  const ref = useRef<HTMLDivElement>(null);
  const onFirstPromptRef = useRef(onFirstPrompt);
  onFirstPromptRef.current = onFirstPrompt;
  const captureRef = useRef(initialCapture());
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [showSearch, setShowSearch] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [dragOver, setDragOver] = useState(false);

  useEffect(() => {
    const container = ref.current;
    if (!container) return;
    const term = new Terminal({ convertEol: true, fontSize: 13, fontFamily: TERMINAL_FONT_FAMILY, cursorBlink: true, theme: currentXtermTheme(), minimumContrastRatio: minContrastRatio(), scrollback: 10000 });
    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    searchAddonRef.current = search;
    termRef.current = term;
    term.open(container);
    // Shift/Ctrl+Enter → insert a newline instead of submitting. xterm sends a bare
    // CR (\r) for Enter regardless of modifiers, so a TUI like Claude Code can't tell
    // "submit" from "newline". We intercept the modified chord and send LF (\n, 0x0a),
    // which is exactly what Claude Code's `/terminal-setup` maps Shift+Enter to. Bare
    // Enter falls through to xterm's default (\r) and still submits.
    term.attachCustomKeyEventHandler((e) => {
      if (
        e.type === "keydown" && e.key === "Enter" &&
        (e.shiftKey || e.ctrlKey) && !e.altKey && !e.metaKey
      ) {
        stream.input(runId, "\n");
        return false; // handled — don't let xterm also emit \r
      }
      return true;
    });
    // Fit xterm to its container, then push the new size to the backend so the
    // PTY (and thus the daemon emulator) reflows to match. resize_run is a no-op until the
    // attach lands, so it's safe to call before/while attaching.
    const doFit = () => {
      try {
        fit.fit();
        if (term.cols > 0 && term.rows > 0) stream.resize(runId, term.cols, term.rows).catch(() => {});
      } catch { /* not laid out */ }
    };
    requestAnimationFrame(() => { doFit(); term.focus(); });
    const ro = new ResizeObserver(doFit);
    ro.observe(container);

    const onThemeChange = () => {
      term.options.theme = currentXtermTheme();
      term.options.minimumContrastRatio = minContrastRatio();
    };
    window.addEventListener("themechange", onThemeChange);

    let disposed = false;
    let liveStarted = false;
    let seeded = false;
    let onData: { dispose(): void } | undefined;
    stream.preview(runId, 200).then((seed) => {
      // Only seed before the live stream lands. Once attach is streaming, the daemon has
      // switched the terminal into its alternate screen and repainted; writing the
      // (now stale) snapshot on top of that corrupts the live screen — e.g. an
      // extra line above the prompt. The live attach is authoritative.
      if (disposed || liveStarted || !seed) return;
      // the daemon snapshot may pad with blank lines up to the pane height;
      // we also used to force a trailing newline. Both rendered as a block of empty
      // lines on every open. Trim trailing blank lines.
      const trimmed = seed.replace(/[\r\n]+$/, "");
      if (trimmed) { term.write(trimmed); seeded = true; }
    });
    (async () => {
      try {
        await stream.ensureActive?.(runId);
      } catch (e) {
        if (!disposed) term.write(`\r\n\x1b[31mCould not resume this run: ${e}\x1b[0m\r\n`);
        return; // don't attach to a session that failed to come up
      }
      if (disposed) return;
      stream.attach(runId, (bytes) => {
        if (!liveStarted) {
          liveStarted = true;
          // The live attach is authoritative: the daemon sends a full snapshot on attach.
          // If we already painted a preview seed, the seed and the repaint overlap
          // (the snapshot is positioned relative to the old screen, the repaint to a
          // fresh one) and leave artifacts — e.g. a stale blank line above the prompt.
          // Resetting first lets the repaint own a clean screen. The guard above also
          // skips the seed when attach wins the race, so this only fires when needed.
          if (seeded) term.reset();
        }
        term.write(bytes);
      }).then(() => {
        if (disposed) return;
        onData = term.onData((d) => {
          stream.input(runId, d);
          const cb = onFirstPromptRef.current;
          if (cb && !captureRef.current.done) {
            const r = feed(captureRef.current, d);
            captureRef.current = r.state;
            if (r.line !== null) cb(r.line);
          }
        });
        // Re-send the size now that the attach exists, so the first paint matches.
        doFit();
      }).catch((e) => {
        // Attach failed: without this the terminal just sits silently blank.
        if (!disposed) term.write(`\r\n\x1b[31mCould not attach to this session: ${e}\x1b[0m\r\n`);
      });
    })();

    return () => {
      disposed = true;
      ro.disconnect();
      window.removeEventListener("themechange", onThemeChange);
      onData?.dispose();
      stream.detach(runId);
      term.dispose();
      captureRef.current = initialCapture();
      searchAddonRef.current = null;
      termRef.current = null;
    };
  }, [runId, stream]);

  // Dropping files/images from Finder. Tauri intercepts OS-level drag-drop at the
  // webview boundary, so HTML5 drop events never reach the terminal div — we listen
  // to Tauri's own drag-drop stream instead. The event is window-global and fires for
  // every mounted terminal, so each instance hit-tests the drop position against its
  // own rect and only the one under the cursor inserts the path(s). Positions arrive in
  // physical pixels; divide by devicePixelRatio to compare with CSS-pixel client rects.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    const hit = (pos: { x: number; y: number }) => {
      const el = ref.current;
      if (!el) return false;
      const dpr = window.devicePixelRatio || 1;
      const x = pos.x / dpr, y = pos.y / dpr;
      const r = el.getBoundingClientRect();
      return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
    };
    // Quote paths with whitespace so a multi-word path lands as one argument.
    const quote = (p: string) => (/\s/.test(p) ? `'${p.replace(/'/g, `'\\''`)}'` : p);
    getCurrentWebview().onDragDropEvent((event) => {
      if (disposed) return;
      const p = event.payload;
      if (p.type === "enter" || p.type === "over") {
        setDragOver(hit(p.position));
      } else if (p.type === "drop") {
        setDragOver(false);
        if (hit(p.position) && p.paths.length) {
          stream.input(runId, p.paths.map(quote).join(" ") + " ");
          termRef.current?.focus();
        }
      } else {
        setDragOver(false);
      }
    }).then((u) => { if (disposed) u(); else unlisten = u; });
    return () => { disposed = true; unlisten?.(); };
  }, [runId, stream]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        // Multiple FocusTerminals mount at once (agent + companion shell). This handler
        // is window-global, so without a guard every instance opens its search box on
        // Cmd+F. Only respond when this instance's xterm container actually holds focus
        // (xterm keeps focus in a .xterm-helper-textarea inside the container).
        if (!ref.current?.contains(document.activeElement)) return;
        e.preventDefault();
        setShowSearch(true);
        requestAnimationFrame(() => searchInputRef.current?.focus());
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const onSearch = (q: string, prev = false) => {
    const addon = searchAddonRef.current;
    if (!addon || !q) return;
    prev ? addon.findPrevious(q) : addon.findNext(q);
  };

  const closeSearch = () => {
    setShowSearch(false);
    setSearchQuery("");
    termRef.current?.focus();
  };

  const handleSearchKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      onSearch(searchQuery, e.shiftKey);
    } else if (e.key === "Escape") {
      closeSearch();
    }
  };

  return (
    <div className="term-search-wrap">
      <div className={`terminal focus-term${dragOver ? " term-drag-over" : ""}`} ref={ref} />
      {showSearch && (
        <div className="term-search-box">
          <input
            ref={searchInputRef}
            className="term-search-input"
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
            onKeyDown={handleSearchKey}
            placeholder="Search…"
          />
          <button className="term-search-btn" onClick={() => onSearch(searchQuery, false)} title="Next (Enter)">↓</button>
          <button className="term-search-btn" onClick={() => onSearch(searchQuery, true)} title="Prev (Shift+Enter)">↑</button>
          <button className="term-search-close" onClick={closeSearch} title="Close (Esc)">✕</button>
        </div>
      )}
    </div>
  );
}
