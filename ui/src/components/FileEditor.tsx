import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { basicSetup } from "codemirror";
import { defaultKeymap } from "@codemirror/commands";
import { openUrl } from "@tauri-apps/plugin-opener";
import { FileRoot, FileStat, readFile, readFileBase64, statFile, writeFile } from "../api";
import { loadLanguage } from "../lib/cmLanguage";
import { editorChromeTheme, editorHighlight } from "../lib/cmTheme";
import { cmFindEngine, cmFindExtensions } from "../lib/cmFind";
import { domFindEngine } from "../lib/domFind";
import { FindEngine } from "../lib/find";
import { FindRank } from "../lib/findBus";
import { useFind } from "../hooks/useFind";
import { getWordWrap } from "../lib/editorPrefs";
import { bufferKey, dropBuffer, stashBuffer, takeBuffer } from "../lib/editorBuffers";
import { mayHaveChanged, minimalChange } from "../lib/diskSync";
import { editorScroll, isMeasurable, previewScroll, trackEditorScroll } from "../lib/scrollMemory";
import { isExternalHref, onMarkdownLinkClick, renderMarkdown } from "../lib/mdHtml";
import { toastError } from "../lib/toast";
import ConfirmDialog from "./ConfirmDialog";
import Menu, { MenuEntry } from "./git/Menu";
import { shortcutLabel } from "../lib/platform";

// How a file is displayed. Raster images, PDFs, and playable audio/video
// render directly (no code view); md/html/svg open in the editor with a
// Preview toggle; everything else is plain text (or "binary — not shown").
type ViewKind =
  | { kind: "image" }
  | { kind: "pdf" }
  | { kind: "audio" }
  | { kind: "video" }
  | { kind: "text"; preview: "md" | "html" | "svg" | null };

// Extensions the browser's <audio>/<video> can actually decode. Container
// formats Chromium can't play (mkv/avi/wmv/flv) are intentionally excluded so
// they fall through to the honest "binary file" message rather than a broken
// player. Keep in step with mime_for() in crates/agency-core/src/files.rs.
const AUDIO_EXTS = ["mp3", "wav", "flac", "aac", "ogg", "oga", "opus", "m4a"];
const VIDEO_EXTS = ["mp4", "m4v", "webm", "mov", "ogv"];

function viewKind(path: string): ViewKind {
  const ext = (path.split(".").pop() ?? "").toLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif"].includes(ext)) return { kind: "image" };
  if (ext === "pdf") return { kind: "pdf" };
  if (AUDIO_EXTS.includes(ext)) return { kind: "audio" };
  if (VIDEO_EXTS.includes(ext)) return { kind: "video" };
  if (ext === "md" || ext === "markdown") return { kind: "text", preview: "md" };
  if (ext === "html" || ext === "htm") return { kind: "text", preview: "html" };
  if (ext === "svg") return { kind: "text", preview: "svg" };
  return { kind: "text", preview: null };
}

export interface FileEditorHandle {
  /** Scroll a 1-based line into view (clamped) and put the cursor there. */
  scrollToLine: (line: number) => void;
  /**
   * Stash the live doc into the buffer cache now (no-op when clean). The owner
   * calls this before a rename retargets the tab, so unsaved edits can migrate
   * to the new buffer key instead of dying with the old editor instance.
   */
  stashIfDirty: () => void;
  /**
   * Forget the dirty state so an imminent unmount does NOT stash the doc.
   * Called when the user explicitly discards (close-without-saving, delete) —
   * without it the unmount stash would resurrect the discarded edits.
   */
  discard: () => void;
}

const FileEditor = forwardRef<FileEditorHandle, {
  root: FileRoot;
  path: string;
  onDirtyChange?: (dirty: boolean) => void;
  /** Open on the rendered preview rather than the code (md/html/svg only). */
  preview?: boolean;
  /** The Code / Preview toggle moved, so the owner can remember it. */
  onPreviewChange?: (preview: boolean) => void;
}>(function FileEditor({ root, path, onDirtyChange, preview, onPreviewChange }, ref) {
  const hostRef = useRef<HTMLDivElement>(null);
  // The whole pane, so the find bar counts as part of this surface when ⌘F
  // works out which one holds focus.
  const wrapRefEl = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  // Compartment so the word-wrap toggle reconfigures the live editor in place.
  const wrapRef = useRef(new Compartment());
  // Ditto for the language: grammars load as dynamic chunks, so the editor
  // mounts on plain text and swaps the grammar in when its chunk arrives.
  const langRef = useRef(new Compartment());
  const [status, setStatus] = useState<"loading" | "binary" | "tooLarge" | "ready" | "error">("loading");
  const [dirty, setDirty] = useState(false);
  // Mirror of `dirty` for unmount cleanup + a change-only owner callback.
  const dirtyRef = useRef(false);
  const onDirtyRef = useRef(onDirtyChange);
  onDirtyRef.current = onDirtyChange;
  const markDirty = (d: boolean) => {
    if (dirtyRef.current !== d) {
      dirtyRef.current = d;
      onDirtyRef.current?.(d);
    }
    setDirty(d);
  };
  const markDirtyRef = useRef(markDirty);
  markDirtyRef.current = markDirty;
  const [errorMsg, setErrorMsg] = useState("");
  // The signature of the disk text the editor holds, from the read that
  // filled it. The poll below stats the file against this and re-reads only
  // when they disagree. Null after our own save: nothing has stat'ed the
  // result, so the next tick re-reads once, finds the text equal, and records
  // the signature then.
  const seenStatRef = useRef<FileStat | null>(null);
  // Binary payload (data URL) for image/pdf files.
  const [dataUrl, setDataUrl] = useState("");
  // Preview toggle for md/html/svg. previewContent holds sanitized HTML for
  // markdown, and the frame/image source (a data: URL) for html and svg.
  const [previewing, setPreviewing] = useState(false);
  const [previewContent, setPreviewContent] = useState("");
  // Read once the file has loaded: the preview renders from the live doc, so
  // it cannot be shown before there is one.
  const wantPreview = useRef(preview ?? false);
  wantPreview.current = preview ?? false;
  const onPreviewRef = useRef(onPreviewChange);
  onPreviewRef.current = onPreviewChange;
  // The rendered markdown itself, for Select all, and the pane that scrolls it.
  const mdRef = useRef<HTMLDivElement>(null);
  const mdScrollRef = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  // Guards the destructive revert behind a confirmation when there are edits.
  const [confirmRevert, setConfirmRevert] = useState(false);

  const vk = viewKind(path);

  // ⌘F / Edit ▸ Find, over whichever view is live in this tab: the code, or
  // the rendered markdown. The markdown preview used to switch ⌘F off along
  // with the html and svg ones, so a rendered README could not be searched
  // without flipping back to the source.
  const previewingRef = useRef(previewing);
  previewingRef.current = previewing;
  const findEngine = useMemo<FindEngine>(() => {
    const code = cmFindEngine(() => viewRef.current);
    const doc = domFindEngine(() => mdRef.current);
    const pick = () => (previewingRef.current ? doc : code);
    return {
      sync: (q) => pick().sync(q),
      recount: (q) => pick().recount(q),
      step: (q, back) => pick().step(q, back),
      replaceOne: (q) => pick().replaceOne(q),
      replaceAll: (q) => pick().replaceAll(q),
      selectedText: () => pick().selectedText(),
      refocus: () => pick().refocus(),
      dismiss: () => pick().dismiss(),
    };
  }, []);
  const searchesPreview = previewing && vk.kind === "text" && vk.preview === "md";
  const { bar: findBar, onContentChange } = useFind({
    host: wrapRefEl,
    engine: findEngine,
    canReplace: !searchesPreview,
    rank: FindRank.editor,
    // Images, PDFs, media and the html/svg previews (a sandboxed frame and an
    // image) have no text we can reach, so ⌘F should reach past them, not
    // open a bar over nothing.
    enabled: vk.kind === "text" && (!previewing || searchesPreview),
    // Switching between code and preview is a different text to search.
    resetKey: previewing ? "preview" : "code",
  });
  const onFindContentChange = useRef(onContentChange);
  onFindContentChange.current = onContentChange;

  // Keep a stable save handler that reads the current doc from the live view.
  const save = useRef(async () => {});
  save.current = async () => {
    const view = viewRef.current;
    if (!view) return;
    try {
      await writeFile(root, path, view.state.doc.toString());
      seenStatRef.current = null;
      markDirtyRef.current(false);
      dropBuffer(bufferKey(root, path));
    } catch (e) {
      setErrorMsg(String(e));
      setStatus("error");
    }
  };

  // Reload the file from disk, discarding in-editor edits. Kept as a ref (like
  // save) so the handler always reads the live view without re-subscribing.
  const revert = useRef(async () => {});
  revert.current = async () => {
    const view = viewRef.current;
    if (!view) return;
    try {
      const fc = await readFile(root, path);
      if (fc.binary || fc.tooLarge) return;
      seenStatRef.current = { mtimeMs: fc.mtimeMs, size: fc.size };
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: fc.text } });
      // The dispatch above flips `dirty` back on via the update listener; clear
      // it after so a freshly-reverted buffer reads as clean.
      markDirtyRef.current(false);
      dropBuffer(bufferKey(root, path));
    } catch (e) {
      setErrorMsg(String(e));
      setStatus("error");
    }
  };

  // The file changed underneath us — edited in Finder, or written by an agent
  // working in the same tree — while its tab sat open. Re-read when the window
  // comes back, which is the moment after the detour that caused it (AGE-162),
  // and while it stays in front, whenever the poll below sees the file move
  // (AGE-228: an agent's edit to the open file showed nothing until the tab
  // was closed and the file reopened, because focus never changed hands).
  //
  // Never over unsaved work: a dirty buffer keeps what was typed, and Revert
  // stays the way to take the disk copy instead. The disk text lands as the
  // smallest single edit that produces it, so the lines above the change stay
  // put, the cursor maps through it, and the view is not scrolled: the user
  // reading the file keeps their place, and a background tab that quietly
  // re-reads doesn't jump when it is next looked at.
  const syncFromDisk = useRef(async () => {});
  syncFromDisk.current = async () => {
    const view = viewRef.current;
    if (!view || dirtyRef.current) return;
    try {
      const fc = await readFile(root, path);
      // Still the same editor, still clean: the read is a round trip, and the
      // user can have started typing in the middle of it.
      if (viewRef.current !== view || dirtyRef.current) return;
      if (fc.binary || fc.tooLarge) return;
      // Recorded before the text comparison, so a re-read that finds nothing
      // new (our own save, a touch) still settles the poll.
      seenStatRef.current = { mtimeMs: fc.mtimeMs, size: fc.size };
      const change = minimalChange(view.state.doc.toString(), fc.text);
      if (change === null) return;
      view.dispatch({ changes: change });
      // The dispatch flips `dirty` on via the update listener; this is the
      // disk's own text, so clear it again.
      markDirtyRef.current(false);
    } catch {
      // The file may have just been deleted or renamed. The tree's own refresh
      // is what says so; an error banner over the last good text would not.
    }
  };

  useEffect(() => {
    const onFocus = () => { void syncFromDisk.current(); };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  // The poll: stat the file on the tree's own two-second cadence and re-read
  // only when the signature moves. A stat is cheap enough for every warm tab,
  // active or not, so switching to a tab shows the current text rather than
  // a tick-old one. Nobody is reading the editor while another app is in
  // front; the focus listener above catches up on that stretch.
  const pollDisk = useRef(async () => {});
  pollDisk.current = async () => {
    const view = viewRef.current;
    if (!view || dirtyRef.current) return;
    try {
      const now = await statFile(root, path);
      if (viewRef.current !== view || dirtyRef.current) return;
      if (mayHaveChanged(seenStatRef.current, now)) await syncFromDisk.current();
    } catch {
      // Gone or renamed: the tree's sweep reports it; the last good text stays.
    }
  };

  useEffect(() => {
    const tick = () => { if (document.hasFocus()) void pollDisk.current(); };
    const t = window.setInterval(tick, 2000);
    return () => window.clearInterval(t);
  }, []);

  useImperativeHandle(ref, () => ({
    scrollToLine: (line: number) => {
      const view = viewRef.current;
      if (!view) return;
      const doc = view.state.doc;
      const l = doc.line(Math.min(Math.max(line, 1), doc.lines));
      view.dispatch({
        selection: { anchor: l.from },
        effects: EditorView.scrollIntoView(l.from, { y: "start", yMargin: 12 }),
      });
      view.focus();
    },
    stashIfDirty: () => {
      if (dirtyRef.current && viewRef.current) {
        stashBuffer(bufferKey(root, path), viewRef.current.state.doc.toString());
      }
    },
    discard: () => {
      dirtyRef.current = false;
    },
  }));

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    markDirtyRef.current(false);
    seenStatRef.current = null;
    setErrorMsg("");
    setDataUrl("");
    setPreviewing(false);
    setPreviewContent("");

    // Images, PDFs, and media skip the text pipeline entirely.
    if (vk.kind === "image" || vk.kind === "pdf" || vk.kind === "audio" || vk.kind === "video") {
      readFileBase64(root, path).then((bc) => {
        if (cancelled) return;
        if (bc.tooLarge) { setStatus("tooLarge"); return; }
        setDataUrl(`data:${bc.mime};base64,${bc.b64}`);
        setStatus("ready");
      }).catch((e) => {
        if (cancelled) return;
        setErrorMsg(String(e));
        setStatus("error");
      });
      return () => { cancelled = true; };
    }

    readFile(root, path).then((fc) => {
      if (cancelled) return;
      if (fc.tooLarge) { setStatus("tooLarge"); return; }
      if (fc.binary) { setStatus("binary"); return; }
      seenStatRef.current = { mtimeMs: fc.mtimeMs, size: fc.size };
      setStatus("ready");
      const host = hostRef.current;
      if (!host) { setStatus("error"); setErrorMsg("Editor failed to mount."); return; }
      viewRef.current?.destroy();
      // An unmount with unsaved edits stashed the doc; restore it over the
      // disk text and stay dirty, so tab/root switches never lose work.
      const stashed = takeBuffer(bufferKey(root, path));
      const state = EditorState.create({
        doc: stashed ?? fc.text,
        extensions: [
          basicSetup,
          cmFindExtensions,
          editorChromeTheme,
          editorHighlight,
          wrapRef.current.of(getWordWrap() ? EditorView.lineWrapping : []),
          langRef.current.of([]),
          keymap.of([
            { key: "Mod-s", preventDefault: true, run: () => { void save.current(); return true; } },
            ...defaultKeymap,
          ]),
          EditorView.updateListener.of((u) => {
            if (!u.docChanged) return;
            markDirtyRef.current(true);
            onFindContentChange.current();
          }),
        ],
      });
      // Back where the reader left this file, if they have been here this
      // session (AGE-253). CodeMirror holds the target until the host is laid
      // out, so the `display: none` it mounts under does not lose it.
      const key = bufferKey(root, path);
      const view = new EditorView({ state, parent: host, scrollTo: editorScroll.recall(key) ?? undefined });
      viewRef.current = view;
      trackEditorScroll(view, key);
      if (stashed !== null) markDirtyRef.current(true);
      if (wantPreview.current && vk.kind === "text" && vk.preview !== null) showPreviewRef.current();
      // The first line lets shebang scripts (scripts/deploy, no extension)
      // resolve. Guarded on the live view so a fast tab switch can't drop a
      // stale grammar into the next file's editor.
      void loadLanguage(path, fc.text.slice(0, 200).split("\n", 1)[0]).then((lang) => {
        if (cancelled || viewRef.current !== view || lang.length === 0) return;
        view.dispatch({ effects: langRef.current.reconfigure(lang) });
      });
    }).catch((e) => {
      if (cancelled) return;
      setErrorMsg(String(e));
      setStatus("error");
    });

    return () => {
      cancelled = true;
      if (dirtyRef.current && viewRef.current) {
        stashBuffer(bufferKey(root, path), viewRef.current.state.doc.toString());
      }
      viewRef.current?.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [root.kind, root.id, path]);

  // Apply word-wrap toggles to the open editor without reloading the file.
  useEffect(() => {
    const onWrap = (e: Event) => {
      const on = (e as CustomEvent<boolean>).detail;
      viewRef.current?.dispatch({
        effects: wrapRef.current.reconfigure(on ? EditorView.lineWrapping : []),
      });
    };
    window.addEventListener("wordwrapchange", onWrap);
    return () => window.removeEventListener("wordwrapchange", onWrap);
  }, []);

  // Render the preview from the live editor doc (unsaved edits included).
  function renderPreview() {
    const doc = viewRef.current?.state.doc.toString() ?? "";
    if (vk.kind === "text" && vk.preview === "md") {
      setPreviewContent(renderMarkdown(doc, { labelFences: true }));
    } else if (vk.kind === "text" && vk.preview === "html") {
      // data: URL, not srcDoc: srcdoc documents inherit the app's strict CSP,
      // which blocks the page's own scripts — and design-handoff HTML is
      // usually script-rendered, so it previewed blank. A data: frame (already
      // allowed by frame-src, same as the PDF path) gets an opaque origin.
      setPreviewContent(`data:text/html;charset=utf-8,${encodeURIComponent(doc)}`);
    } else if (vk.kind === "text" && vk.preview === "svg") {
      setPreviewContent(`data:image/svg+xml;charset=utf-8,${encodeURIComponent(doc)}`);
    }
    setPreviewing(true);
  }
  const showPreviewRef = useRef(renderPreview);
  showPreviewRef.current = renderPreview;

  // Each toggle clears the side being left first: the bar closes on the
  // switch, and match paint left in a hidden editor would reappear with no bar
  // to press Escape in.
  const showPreview = () => {
    findEngine.dismiss();
    renderPreview();
    onPreviewRef.current?.(true);
  };
  const showCode = () => {
    findEngine.dismiss();
    setPreviewing(false);
    onPreviewRef.current?.(false);
  };

  // The rendered markdown is rebuilt on every switch to it, so it would open at
  // the top each time: put it back where it was read to (AGE-253).
  useLayoutEffect(() => {
    const el = mdScrollRef.current;
    if (!el || !previewing || !previewContent || status !== "ready") return;
    el.scrollTop = previewScroll.recall(bufferKey(root, path)) ?? 0;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [previewing, previewContent, status]);

  // A re-render replaces the preview's DOM, and with it the match marks.
  useEffect(() => { onContentChange(); }, [previewContent, onContentChange]);

  // ── Markdown preview right-click menu ────────────────────────────────────
  // The rendered preview is read-only, so this is the clipboard menu a
  // document is expected to carry, plus the way back to the source.

  const copyText = async (text: string, failure: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      toastError(e, failure);
    }
  };

  const selectAllPreview = () => {
    const el = mdRef.current;
    const sel = window.getSelection();
    if (!el || !sel) return;
    const range = document.createRange();
    range.selectNodeContents(el);
    sel.removeAllRanges();
    sel.addRange(range);
  };

  const openPreviewMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    // getSelection() is the whole window's: a selection left behind in another
    // pane would otherwise arm Copy here and copy that pane's text.
    const sel = window.getSelection();
    const inPreview = !!sel && !sel.isCollapsed && !!mdRef.current?.contains(sel.anchorNode);
    const selected = inPreview ? sel.toString() : "";
    const href = (e.target as HTMLElement).closest("a")?.getAttribute("href") ?? "";
    const items: MenuEntry[] = [];
    if (isExternalHref(href)) {
      items.push(
        {
          label: "Open link in browser",
          onClick: () => void openUrl(href).catch((err) => toastError(err, "Couldn't open the link")),
        },
        { label: "Copy link", onClick: () => void copyText(href, "Couldn't copy the link") },
        { kind: "separator" },
      );
    }
    items.push(
      // No hint on Select all: the preview isn't a focusable text field, so ⌘A
      // reaches the app, not this pane.
      { label: "Copy", hint: shortcutLabel("⌘C"), disabled: !selected, onClick: () => void copyText(selected, "Couldn't copy") },
      { label: "Select all", onClick: selectAllPreview },
      { kind: "separator" },
      { label: "Show source", onClick: showCode },
    );
    setMenu({ x: e.clientX, y: e.clientY, items });
  };

  const previewable = vk.kind === "text" && vk.preview !== null;

  return (
    <div className="file-editor-wrap" ref={wrapRefEl}>
      {findBar}
      <div className="file-editor-bar">
        <span className="file-editor-path" title={path}>{path}</span>
        {dirty && <span className="file-editor-dirty" title="Unsaved changes">●</span>}
        <span className="spacer" style={{ flex: 1 }} />
        {previewable && status === "ready" && (
          <div className="seg seg-mini">
            <button className={previewing ? "" : "on"} onClick={showCode}>Code</button>
            <button className={previewing ? "on" : ""} onClick={showPreview}>Preview</button>
          </div>
        )}
        {vk.kind === "text" && (
          <>
            <button className="file-editor-btn" title="Revert changes" aria-label="Revert changes"
              disabled={!dirty || status !== "ready"} onClick={() => setConfirmRevert(true)}>
              <RevertGlyph />
            </button>
            <button className="file-editor-btn" title={`Save (${shortcutLabel("⌘S")})`} aria-label="Save"
              disabled={!dirty || status !== "ready"} onClick={() => void save.current()}>
              <SaveGlyph />
            </button>
          </>
        )}
      </div>
      {status === "loading" && <div className="diff-empty">loading…</div>}
      {status === "binary" && <div className="diff-empty">Binary file. No preview for this format.</div>}
      {status === "tooLarge" && <div className="diff-empty">File too large to open.</div>}
      {status === "error" && <div className="git-error">{errorMsg}</div>}

      {vk.kind === "image" && status === "ready" && (
        <div className="file-preview-media"><img src={dataUrl} alt={path} /></div>
      )}
      {vk.kind === "pdf" && status === "ready" && (
        <iframe className="file-preview-frame" title={path} src={dataUrl} />
      )}
      {vk.kind === "audio" && status === "ready" && (
        <div className="file-preview-media"><audio controls src={dataUrl} /></div>
      )}
      {vk.kind === "video" && status === "ready" && (
        <div className="file-preview-media"><video controls src={dataUrl} /></div>
      )}

      {vk.kind === "text" && (
        <>
          <div
            ref={hostRef}
            className="file-editor-host"
            style={{ display: status === "ready" && !previewing ? "block" : "none" }}
          />
          {previewing && status === "ready" && (
            vk.preview === "svg" ? (
              <div className="file-preview-media"><img src={previewContent} alt={path} /></div>
            ) : vk.preview === "html" ? (
              // allow-scripts WITHOUT allow-same-origin: the page's own JS runs
              // (script-rendered HTML previews correctly) but the opaque data:
              // origin has no same-origin or IPC access to the app.
              <iframe className="file-preview-frame" title={path} sandbox="allow-scripts" src={previewContent} />
            ) : (
              // Markdown renders in the app's own document, not a frame. A
              // frame is a separate document that the app's right-click
              // suppression (main.tsx) cannot reach into, so WebKit offered its
              // own menu there, carrying the single item it has for a subframe:
              // "Open Frame in New Window", which the sandbox then refuses to
              // act on. That is AGE-156. DOMPurify plus the app CSP are the
              // same two guards the PR comment bodies rely on (lib/mdHtml).
              <div
                ref={mdScrollRef}
                className="file-preview-doc"
                onContextMenu={openPreviewMenu}
                onScroll={(e) => {
                  const el = e.currentTarget;
                  if (isMeasurable(el)) previewScroll.remember(bufferKey(root, path), el.scrollTop);
                }}
              >
                <div
                  ref={mdRef}
                  className="file-preview-md"
                  onClick={onMarkdownLinkClick}
                  dangerouslySetInnerHTML={{ __html: previewContent }}
                />
              </div>
            )
          )}
        </>
      )}

      {confirmRevert && (
        <ConfirmDialog
          title="Revert changes?"
          body={`Discard unsaved changes to ${path.split("/").pop()} and reload it from disk. This can't be undone.`}
          confirmLabel="Revert"
          danger
          onConfirm={() => { setConfirmRevert(false); void revert.current(); }}
          onCancel={() => setConfirmRevert(false)}
        />
      )}

      {menu && <Menu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
});

export default FileEditor;

// ── Toolbar glyphs (stroked SVG, matching the app's icon convention) ──────
const eg = {
  width: 15, height: 15, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor",
  strokeWidth: 1.9, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, "aria-hidden": true,
};
// Floppy-disk save glyph.
function SaveGlyph() {
  return (
    <svg {...eg}>
      <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
      <polyline points="17 21 17 13 7 13 7 21" />
      <polyline points="7 3 7 8 15 8" />
    </svg>
  );
}
// Curved undo arrow: discard edits / reload from disk.
function RevertGlyph() {
  return (
    <svg {...eg}>
      <path d="M9 14 4 9l5-5" />
      <path d="M4 9h11a5 5 0 0 1 0 10h-4" />
    </svg>
  );
}
