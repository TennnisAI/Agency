import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import "@xterm/xterm/css/xterm.css";
import { attachRun, detachRun, resizeRun, runInput, runPreview, ensureRunActive,
  attachRunScript, detachRunScript, resizeRunScript, runScriptInput, runScriptPreview } from "../api";
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
    // Fit xterm to its container, then push the new size to the backend so the
    // PTY (and thus tmux) reflows to match. resize_run is a no-op until the
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
      // Only seed before the live stream lands. Once attach is streaming, tmux has
      // switched the terminal into its alternate screen and repainted; writing the
      // (now stale) snapshot on top of that corrupts the live screen — e.g. an
      // extra line above the prompt. The live attach is authoritative.
      if (disposed || liveStarted || !seed) return;
      // tmux capture-pane pads the snapshot with blank lines up to the pane height;
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
          // The live attach is authoritative: tmux sends a full repaint on attach.
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

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
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
      <div className="terminal focus-term" ref={ref} />
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
