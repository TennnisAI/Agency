import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import "@xterm/xterm/css/xterm.css";
import { attachRun, detachRun, resizeRun, runInput, runPreview, ensureRunActive,
  attachRunScript, detachRunScript, resizeRunScript, runScriptInput, runScriptPreview,
  attachShell, detachShell, resizeShell, shellInput, shellPreview, startShell } from "../api";
import { useRuns } from "../store/runs";
import { currentXtermTheme, minContrastRatio, TERMINAL_FONT_FAMILY } from "../lib/themes";
import { initialCapture, feed } from "../lib/firstPrompt";
import { shouldSwallowWheel, createPageScroller } from "../lib/termScroll";
import { follow, GESTURE_MS } from "../lib/termFollow";
import { createInputWriter, type InputWriter } from "../lib/termInput";
import { fullClipboardText } from "../lib/clipboard";
import { FindRank, registerFindTarget } from "../lib/findBus";

export interface TerminalStream {
  attach(id: string, cols: number, rows: number, onBytes: (b: Uint8Array) => void): Promise<void>;
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
  // `altScrollArrows` allows xterm's alternate-scroll fallback (wheel notch ->
  // Up/Down arrow) for panes running a pager or an editor. It must be off for a
  // pane running an agent, where those arrows walk the prompt history. It is a
  // property of the pane, not of the stream: a "New terminal" tab is a shell on
  // the same `agentStream` as an agent tab.
  { runId, stream = agentStream, onFirstPrompt, altScrollArrows = true }:
    { runId: string; stream?: TerminalStream; onFirstPrompt?: (line: string) => void; altScrollArrows?: boolean },
) {
  const ref = useRef<HTMLDivElement>(null);
  const onFirstPromptRef = useRef(onFirstPrompt);
  onFirstPromptRef.current = onFirstPrompt;
  // Read through a ref: the wheel handler is installed once, with the terminal.
  const altScrollRef = useRef(altScrollArrows);
  altScrollRef.current = altScrollArrows;
  const captureRef = useRef(initialCapture());
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  // The terminal plus its search box — what "does this surface have focus?"
  // has to cover once ⌘F is routed centrally (see lib/findBus).
  const wrapRef = useRef<HTMLDivElement>(null);
  // Claims a viewport move as deliberate for the follow policy (lib/termFollow).
  // Held on the component so search, which scrolls to its match, can claim one
  // too instead of being pinned straight back down. Installed by the terminal
  // effect; a no-op until then.
  const markGestureRef = useRef<() => void>(() => {});
  // Everything this pane types at its session goes through here, so a frame's
  // worth of it travels as one write (see lib/termInput). Owned by the terminal
  // effect; the drag-drop effect, which has a mount of its own, writes through
  // the ref.
  const inputRef = useRef<InputWriter | null>(null);
  const [showSearch, setShowSearch] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [dragOver, setDragOver] = useState(false);

  // Tell the shell which run is actually visible, so notifications can skip it
  // (see `onScreenRunId`). Only the agent pane counts: a run's companion shell
  // or dev-server pane doesn't show its agent's turn ending. The functional
  // clear keeps a pane swap (old unmounts after the new one mounts) from
  // blanking the run that just took over.
  const { setOnScreenRun } = useRuns();
  const isAgentPane = stream === agentStream;
  useEffect(() => {
    if (!isAgentPane) return;
    setOnScreenRun(runId);
    return () => setOnScreenRun((cur) => (cur === runId ? null : cur));
  }, [isAgentPane, runId, setOnScreenRun]);

  useEffect(() => {
    const container = ref.current;
    if (!container) return;
    // The catch covers the frame of slack the writer buys: a write can now land
    // just after the session it was typed into went away (the pane is closing,
    // the run was discarded), and "you typed at a terminal that is no longer
    // there" is not something to raise a toast over.
    const input = createInputWriter((data) => { stream.input(runId, data).catch(() => {}); });
    inputRef.current = input;
    // Large scrollback: this is the live scroll depth (xterm accumulates the
    // streamed output into its own buffer), so a small cap is what makes long
    // agent conversations "stop" scrolling well before their start. xterm
    // allocates lines lazily, so the ceiling only costs memory once it's filled.
    const term = new Terminal({ convertEol: true, fontSize: 13, fontFamily: TERMINAL_FONT_FAMILY, cursorBlink: true, theme: currentXtermTheme(), minimumContrastRatio: minContrastRatio(), scrollback: 50000 });
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
        // Returning false only makes xterm skip the event; WebKit's own default
        // for Ctrl+Return (show the context menu) still runs without this.
        e.preventDefault();
        input.write("\n");
        return false; // handled — don't let xterm also emit \r
      }
      return true;
    });
    // Scroll wheel. An agent tracking the mouse scrolls itself from the notch,
    // so this handler steps aside and lets xterm deliver it (skipping that is
    // what made agent panes sluggish, AGE-26). It only takes over where xterm
    // would otherwise fall back to alternate-scroll arrows, which an agent
    // prompt reads as history: send the page keys the agent asks for instead.
    // That is also the path a pane gets from a daemon predating the snapshot
    // fix that restores mouse reporting (see term/protocol.rs), so the
    // daemon-side fix can roll out without force-replacing a running daemon.
    const pageScroll = createPageScroller();
    term.attachCustomWheelEventHandler((e) => {
      if (shouldSwallowWheel(term.buffer.active.type, term.modes.mouseTrackingMode, altScrollRef.current)) {
        e.preventDefault(); // xterm skips its own default but not the browser's
        const keys = pageScroll(e.deltaY, e.deltaMode);
        if (keys) input.write(keys);
        return false;
      }
      return true;
    });
    // Stay on the newest output (see lib/termFollow). One scroll the user never
    // made parks a pane for good: xterm latches "the user is scrolling" and only
    // a scroll that lands back at the bottom clears it, so the pane keeps the
    // window it was left on while the agent writes past it, and each further
    // phantom drags it further back — eventually onto the opening prompt with
    // the agent still working (AGE-19). So a scroll counts as the user's only if
    // a gesture is behind it, and the pane is pinned back down otherwise, which
    // clears the latch with it.
    let following = true;
    let dragging = false;
    let repinning = false;
    let gestureAt = -Infinity;
    let torn = false;
    const timers: number[] = [];
    const atBottom = () => {
      const buf = term.buffer.active;
      return buf.viewportY >= buf.baseY;
    };
    // xterm suppresses the scroll event for everything the viewport drives, so
    // a wheel or a drag is never reported: read the position once the gesture
    // has landed instead. Twice, a frame apart, since the DOM scroll it goes
    // through can be a frame late.
    const sample = () => { if (!torn) following = atBottom(); };
    const markGesture = () => {
      gestureAt = performance.now();
      requestAnimationFrame(sample);
      timers.push(window.setTimeout(sample, GESTURE_MS));
    };
    markGestureRef.current = markGesture;
    const onPointerDown = () => { dragging = true; markGesture(); };
    // The gesture outlives the button: a drag-selection past the top edge keeps
    // auto-scrolling for as long as it is held.
    const onPointerUp = () => { dragging = false; markGesture(); };
    const onScrollKey = (e: KeyboardEvent) => {
      if (e.key === "PageUp" || e.key === "PageDown" || e.key === "Home" || e.key === "End") markGesture();
    };
    container.addEventListener("wheel", markGesture, { capture: true, passive: true });
    container.addEventListener("touchstart", markGesture, { capture: true, passive: true });
    container.addEventListener("touchmove", markGesture, { capture: true, passive: true });
    container.addEventListener("keydown", onScrollKey, true);
    container.addEventListener("pointerdown", onPointerDown, true);
    // On window: a drag that ends outside the pane still has to release it.
    window.addEventListener("pointerup", onPointerUp, true);
    window.addEventListener("pointercancel", onPointerUp, true);
    // Fires when output scrolls the buffer, which is where a pane that was left
    // behind finds out: the phantom scroll itself is silent.
    const scrollWatch = term.onScroll(() => {
      if (repinning) return; // our own scrollToBottom, re-entering this handler
      const buf = term.buffer.active;
      const next = follow({
        viewportY: buf.viewportY,
        baseY: buf.baseY,
        gesture: dragging || performance.now() - gestureAt < GESTURE_MS,
        following,
      });
      following = next.following;
      if (!next.repin) return;
      repinning = true;
      try {
        term.scrollToBottom();
      } finally {
        repinning = false;
      }
    });

    // Paste. xterm pastes whatever the paste event's `clipboardData` holds, and
    // WebKit fills that from the first pasteboard item alone: a multi-row copy
    // out of Xcode (one item per row) reaches the pane as its first line only,
    // and a clipboard carrying file URLs arrives as "Files" with no text at all
    // (see lib/clipboard.ts). So read the whole pasteboard from the backend and
    // paste that. `term.paste` still frames it as a bracketed paste, so the
    // agent sees one paste rather than a line of typing per row.
    const onPaste = (e: ClipboardEvent) => {
      const fromWebview = e.clipboardData?.getData("text/plain") ?? "";
      e.preventDefault();
      e.stopPropagation(); // xterm's own handler would paste the truncated text
      void fullClipboardText(fromWebview).then((text) => {
        if (!torn && text) term.paste(text);
      });
    };
    container.addEventListener("paste", onPaste, true);

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
    // Fit on the next frame, never inside the observer callback. `fit.fit()`
    // resizes the very element being observed, so doing it synchronously feeds
    // the observer its own output and the loop overruns the frame — WebKit then
    // reports "ResizeObserver loop completed with undelivered notifications" as
    // a window error (AGE-34: it surfaced as an error toast whenever the Run
    // pane opened alongside the preview). Deferring keeps each fit in its own
    // frame, and coalescing means a drag-resize fits once per frame, not per
    // pixel.
    let fitFrame = 0;
    const scheduleFit = () => {
      if (fitFrame) return;
      fitFrame = requestAnimationFrame(() => { fitFrame = 0; doFit(); });
    };
    const ro = new ResizeObserver(scheduleFit);
    ro.observe(container);

    const onThemeChange = () => {
      term.options.theme = currentXtermTheme();
      term.options.minimumContrastRatio = minContrastRatio();
    };
    window.addEventListener("themechange", onThemeChange);

    let disposed = false;
    let liveStarted = false;
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
      if (trimmed) term.write(trimmed);
    });
    (async () => {
      try {
        await stream.ensureActive?.(runId);
      } catch (e) {
        if (!disposed) term.write(`\r\n\x1b[31mCould not resume this run: ${e}\x1b[0m\r\n`);
        return; // don't attach to a session that failed to come up
      }
      if (disposed) return;
      // Fit *before* attaching and hand the real dims to the daemon, so its
      // snapshot is generated at the exact geometry xterm is showing. Attaching at
      // a placeholder size made the snapshot paint at the wrong width and the
      // follow-up resize then reflowed it — stacking a second, mis-aligned frame
      // ("decomposed" output that a switch-away-and-back sometimes cleared).
      doFit();
      const cols = term.cols > 0 ? term.cols : 80;
      const rows = term.rows > 0 ? term.rows : 24;
      stream.attach(runId, cols, rows, (bytes) => {
        if (!liveStarted) {
          liveStarted = true;
          // The live attach is authoritative: the daemon sends a full snapshot as its
          // first frame. Reset first so that snapshot owns a clean screen — this clears
          // any preview seed we painted, and (for alt-screen apps, whose snapshot only
          // switches buffers without wiping the normal one) any seed residue too.
          term.reset();
        }
        term.write(bytes);
      }).then(() => {
        if (disposed) return;
        onData = term.onData((d) => {
          input.write(d);
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
      if (fitFrame) cancelAnimationFrame(fitFrame);
      window.removeEventListener("themechange", onThemeChange);
      torn = true;
      timers.forEach(clearTimeout);
      container.removeEventListener("wheel", markGesture, true);
      container.removeEventListener("touchstart", markGesture, true);
      container.removeEventListener("touchmove", markGesture, true);
      container.removeEventListener("keydown", onScrollKey, true);
      container.removeEventListener("pointerdown", onPointerDown, true);
      container.removeEventListener("paste", onPaste, true);
      window.removeEventListener("pointerup", onPointerUp, true);
      window.removeEventListener("pointercancel", onPointerUp, true);
      scrollWatch.dispose();
      onData?.dispose();
      // Before the detach, so a keystroke queued on this frame still reaches
      // the session rather than dying with the pane.
      input.dispose();
      inputRef.current = null;
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
  // own rect and only the one under the cursor inserts the path(s). The payload says
  // PhysicalPosition, but on macOS wry reports NSView coordinates (logical points) and
  // tauri-runtime-wry wraps them unscaled — so the values are already CSS pixels.
  // Dividing by devicePixelRatio here halved them on Retina displays and made the
  // hit-test miss the terminal entirely (drops silently did nothing).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    const hit = (pos: { x: number; y: number }) => {
      const el = ref.current;
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return pos.x >= r.left && pos.x <= r.right && pos.y >= r.top && pos.y <= r.bottom;
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
          inputRef.current?.write(p.paths.map(quote).join(" ") + " ");
          termRef.current?.focus();
        }
      } else {
        setDragOver(false);
      }
    }).then((u) => { if (disposed) u(); else unlisten = u; });
    return () => { disposed = true; unlisten?.(); };
  }, [runId, stream]);

  const onSearch = (q: string, prev = false) => {
    const addon = searchAddonRef.current;
    if (!addon || !q) return;
    // Jumping to a match is a scroll the user asked for: claim it, or the
    // follow policy reads it as drift and pins the pane straight back down.
    markGestureRef.current();
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

  // ⌘F belongs to the Edit menu now, and a menu click carries no DOM target,
  // so this pane registers rather than listening for the key itself.
  // `requiresFocus` is the same guard the old handler had: several terminals
  // are mounted at once (agent grid, companion shell) and only the one that
  // was clicked into should answer. Scrollback has nothing to replace.
  const findRef = useRef<{ open: () => void; step: (back: boolean) => void }>({ open: () => {}, step: () => {} });
  findRef.current = {
    open: () => {
      setShowSearch(true);
      requestAnimationFrame(() => {
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      });
    },
    step: (back: boolean) => onSearch(searchQuery, back),
  };
  useEffect(() => registerFindTarget({
    host: () => wrapRef.current,
    open: () => findRef.current.open(),
    step: (back) => findRef.current.step(back),
    canReplace: false,
    rank: FindRank.terminal,
    requiresFocus: true,
  }), []);

  return (
    <div className="term-search-wrap" ref={wrapRef}>
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
