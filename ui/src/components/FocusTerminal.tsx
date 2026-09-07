import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import "@xterm/xterm/css/xterm.css";
import { openUrl } from "@tauri-apps/plugin-opener";
import { attachRun, detachRun, resizeRun, runInput, runPreview, ensureRunActive,
  attachRunScript, detachRunScript, resizeRunScript, runScriptInput, runScriptPreview,
  attachShell, detachShell, resizeShell, shellInput, shellPreview, startShell,
  FileRoot, openTermPath } from "../api";
import { useRuns } from "../store/runs";
import { currentXtermTheme, minContrastRatio, paperSurface, TERMINAL_FONT_FAMILY } from "../lib/themes";
import { initialCapture, feed } from "../lib/firstPrompt";
import { shouldSwallowWheel, createPageScroller } from "../lib/termScroll";
import { follow, GESTURE_MS } from "../lib/termFollow";
import { createInputWriter, type InputWriter } from "../lib/termInput";
import { createOutputWriter } from "../lib/termOutput";
import { createRecolor, recolor } from "../lib/termPaper";
import { fullClipboardText } from "../lib/clipboard";
import { FindRank, registerFindTarget } from "../lib/findBus";
import { installTermLinks } from "../lib/termLinkProvider";
import { fileRootKey, requestOpenFile } from "../lib/openFile";
import { focusReport } from "../lib/terminalFocus";
import { pathsToInput, registerPathSink } from "../lib/pathDrop";
import { toastError } from "../lib/toast";

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
  const { setOnScreenRun, setTab, selectedRunId, selectedProjectId } = useRuns();
  // Where a clicked path resolves, and where it opens: the root the Files tab
  // is showing. Deliberately that one and not the pane's own session — an open
  // request scoped to any other root is dropped by the view that has to serve
  // it, and in the focus view (where an agent works) the two are the same root
  // anyway. Read through a ref so switching project doesn't rebuild the pane.
  const filesRoot: FileRoot | null = selectedRunId
    ? { kind: "run", id: selectedRunId }
    : selectedProjectId
      ? { kind: "project", id: selectedProjectId }
      : null;
  const linkCtx = useRef({ filesRoot, setTab });
  linkCtx.current = { filesRoot, setTab };
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
    // URLs and file paths in the output, ⌘-clickable (see lib/termLinkProvider).
    // After `open`: the provider listens on the screen element xterm builds there.
    const links = installTermLinks(term, container, {
      root: () => linkCtx.current.filesRoot,
      openUrl: (url) => { openUrl(url).catch((e) => toastError(e, "Couldn't open that link")); },
      openPath: (path, line, hit) => {
        const root = linkCtx.current.filesRoot;
        if (!root) return;
        // A file inside the root opens in the app; a directory, or anything
        // living outside it, belongs to the OS.
        if (hit?.relPath && !hit.isDir) {
          linkCtx.current.setTab("files");
          requestOpenFile({ rootKey: fileRootKey(root), path: hit.relPath, line });
          return;
        }
        openTermPath(root, path).catch((e) => toastError(e, "Couldn't open that path"));
      },
    });
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
    // Judge where the viewport has ended up, wherever the move came from.
    const settle = () => {
      if (repinning || torn) return; // our own scrollToBottom, re-entering this
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
    };
    // Fires when output scrolls the buffer, which is one of the two ways a pane
    // that was left behind finds out.
    const scrollWatch = term.onScroll(settle);
    // The other way, and the only one an idle pane has. `onScroll` above stays
    // silent for everything the viewport itself drives — xterm suppresses that
    // event so the viewport is never fed its own scrolls — so a pane hears
    // about a phantom scroll on the next line of output and not before. A pane
    // sitting at a shell prompt has no next line: whatever moved it stays, and
    // the latch it set stays with it, which is the companion terminal drifting
    // up the moment focus goes to the agent pane and refusing to come back
    // (AGE-88). The viewport element's own DOM scroll event is not suppressed,
    // so take the position from that too. xterm registered its handler on this
    // element first, in the Viewport constructor, so the buffer has already
    // been moved to match by the time this one runs.
    const viewportEl = container.querySelector(".xterm-viewport");
    viewportEl?.addEventListener("scroll", settle, { passive: true });

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
      // A reflow rewrites the buffer under the viewport: rows change, lines are
      // pulled out of the scrollback or pushed into it, and the pane can come
      // out of it sitting above the newest output with no scroll event to say
      // so. Re-judge the position rather than wait for one.
      settle();
    };
    const firstFrame = requestAnimationFrame(() => { doFit(); term.focus(); });
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
    // On a light theme, the dark backgrounds an agent paints go onto paper
    // (see lib/termPaper). One filter per pane, over the live stream only: it
    // carries a partial sequence between chunks, and the daemon's snapshot
    // below is a whole frame on its own.
    const paper = createRecolor(paperSurface);
    // Everything after the snapshot lands here first, so a frame's worth of it
    // reaches xterm as one write and repaints once (see lib/termOutput).
    const output = createOutputWriter((bytes) => {
      // Detach is a command, not a switch: the daemon keeps streaming until it
      // lands, so a chunk can still arrive after this pane is gone, and this one
      // was already a frame behind. Writing it would repaint a terminal nobody
      // can see.
      if (disposed) return;
      term.write(paper(bytes));
    });
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
        // Detach is a command, not a switch: the daemon keeps streaming until it
        // lands, so a chunk can still arrive after this pane is gone. Writing it
        // would repaint a terminal nobody can see, and `reset` below would arm
        // the frame the teardown goes out of its way to let pass.
        if (disposed) return;
        if (!liveStarted) {
          liveStarted = true;
          // The live attach is authoritative: the daemon sends a full snapshot as its
          // first frame. Reset first so that snapshot owns a clean screen — this clears
          // any preview seed we painted, and (for alt-screen apps, whose snapshot only
          // switches buffers without wiping the normal one) any seed residue too.
          term.reset();
          // AGE-147: replay that first frame with the pane's input muted. It is a
          // repaint of history, but xterm answers parts of what it parses and the
          // answer arrives at the child as if it had been typed — today an
          // unsolicited focus report off the snapshot's `?1004h`, tomorrow the reply
          // to any query a replayed stream carries. `write`'s callback runs once the
          // chunk has been parsed, which is the only point at which xterm is done
          // answering it; anything earlier would unmute mid-replay.
          //
          // The snapshot goes in by itself rather than through the frame batcher
          // below, which keeps both halves honest: the mute covers the replay and
          // nothing else, and `liveStarted` still flips the moment the first live
          // bytes exist, so the preview seed above cannot paint over them.
          const resume = input.suspend();
          term.write(recolor(bytes, paperSurface()), () => {
            resume();
            // Then say where focus actually is, which the mute has just
            // swallowed the terminal's own answer to. A child that tracks focus
            // hides its text cursor when told the pane lost it, and this pane is
            // the one that was navigated back to: nothing else will ever tell it
            // otherwise, because a terminal built already holding focus fires no
            // focus event of its own (see lib/terminalFocus, AGE-171).
            if (disposed) return;
            const report = focusReport(
              term.modes.sendFocusMode,
              document.activeElement === term.textarea,
            );
            if (report) input.write(report);
          });
          return;
        }
        output.write(bytes);
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
      cancelAnimationFrame(firstFrame);
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
      viewportEl?.removeEventListener("scroll", settle);
      links();
      scrollWatch.dispose();
      onData?.dispose();
      // Before the detach, so a keystroke queued on this frame still reaches
      // the session rather than dying with the pane.
      input.dispose();
      inputRef.current = null;
      output.dispose();
      stream.detach(runId);
      // xterm's teardown trails the pane's by a frame, deliberately. Work it
      // queued before now still has to run: `term.reset()` on the first live
      // frame calls Viewport.reset, which schedules a syncScrollArea on a raw
      // requestAnimationFrame it never cancels, and that reads the render
      // service's dimensions. Disposing inside that frame pulls the renderer
      // out from under the read, and it surfaces as an error toast over a pane
      // the user has already left: "undefined is not an object (evaluating
      // 'this._renderer.value.dimensions')" (AGE-124).
      //
      // A frame alone was not enough, and the second source is the write
      // buffer. `WriteBuffer` is not a disposable and has no disposal check at
      // all (xterm 5.5.0): a chunk it cannot parse inside its 12ms slice
      // re-schedules the rest on a bare `setTimeout`, which keeps parsing
      // however long that takes and whatever happened to the terminal
      // meanwhile. The daemon's first live frame is a whole-screen snapshot —
      // the largest single write a pane ever takes — so leaving a run while one
      // lands is exactly the case that overruns. Parsing past the dispose moves
      // the cursor, `onCursorMove` reaches `_syncTextArea`, and that reads the
      // same dimensions off a renderer that is gone.
      //
      // So the wait is on xterm being quiet on both counts: an empty write
      // queues behind whatever is still parsing and its callback is the
      // parser-is-idle signal, and the frame after it lets the viewport's own
      // rAF land first. Everything above is torn down synchronously; only the
      // xterm object waits. The timeout is the backstop for a window that has
      // stopped animating — occluded, minimised, on another Space — where no
      // frame arrives and the terminal, 50k lines of scrollback and all, would
      // otherwise never be freed.
      //
      // Ordering around that callback is not enough on its own, because the
      // backstop can win the race it is arranged around: a timer that comes due
      // while the main thread is blocked runs before the frame does, and opening
      // a pane blocks it through a spawn, an attach and a first paint. (Under
      // StrictMode's dev-only double mount, that is a disposal on the way *in*,
      // which is where AGE-124 was seen again: the toast landed over the
      // terminal that had just opened.) So mute the callback as well as ordering
      // around it. `_core` is the same private handle FitAddon reaches through,
      // and a pane being torn down has no viewport left to sync.
      const core = (term as unknown as {
        _core?: { viewport?: { syncScrollArea?: () => void } };
      })._core;
      if (core?.viewport?.syncScrollArea) core.viewport.syncScrollArea = () => {};
      let termDisposed = false;
      const disposeTerm = () => {
        if (termDisposed) return;
        termDisposed = true;
        term.dispose();
      };
      term.write("", () => requestAnimationFrame(disposeTerm));
      window.setTimeout(disposeTerm, 2000);
      captureRef.current = initialCapture();
      searchAddonRef.current = null;
      termRef.current = null;
    };
  }, [runId, stream]);

  // What a dropped path does, wherever it came from: type it at the prompt and
  // put the cursor back in the pane. Held in a ref so the two drop routes (the
  // OS stream, the in-app sink) share one write instead of drifting apart.
  const insertPaths = useRef((paths: string[]) => {
    if (paths.length === 0) return;
    inputRef.current?.write(pathsToInput(paths));
    termRef.current?.focus();
  });

  // The in-app half of that gesture: a row dragged out of the Files or Docs
  // sidebar beside this pane (AGE-200). Tauri's stream below carries OS drags
  // only, so the tree tracks its own drag and hands the path over here.
  // `altScrollArrows` is off exactly for panes running an agent, which is the
  // distinction the hint wants to draw.
  const dropLabel = altScrollArrows ? "the terminal" : "the agent";
  useEffect(() => registerPathSink({
    host: () => ref.current,
    label: dropLabel,
    accept: (paths) => insertPaths.current(paths),
    setOver: setDragOver,
  }), [dropLabel]);

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
    getCurrentWebview().onDragDropEvent((event) => {
      if (disposed) return;
      const p = event.payload;
      if (p.type === "enter" || p.type === "over") {
        setDragOver(hit(p.position));
      } else if (p.type === "drop") {
        setDragOver(false);
        if (hit(p.position) && p.paths.length) {
          insertPaths.current(p.paths);
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

  // The query outlives the box on purpose: ⌘G keeps walking the scrollback
  // after Escape, and reopening selects what's there so typing still replaces
  // it.
  const closeSearch = () => {
    setShowSearch(false);
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
  const openSearch = () => {
    setShowSearch(true);
    requestAnimationFrame(() => {
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    });
  };
  findRef.current = {
    open: openSearch,
    // Nothing searched yet, so Find Next means "start a search" rather than a
    // keypress that does nothing.
    step: (back: boolean) => (searchQuery ? onSearch(searchQuery, back) : openSearch()),
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
