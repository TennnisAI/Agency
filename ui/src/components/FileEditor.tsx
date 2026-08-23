import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { basicSetup } from "codemirror";
import { defaultKeymap } from "@codemirror/commands";
import { openUrl } from "@tauri-apps/plugin-opener";
import { FileRoot, readFile, readFileBase64, writeFile } from "../api";
import { loadLanguage } from "../lib/cmLanguage";
import { editorChromeTheme, editorHighlight } from "../lib/cmTheme";
import { cmFindEngine, cmFindExtensions } from "../lib/cmFind";
import { FindRank } from "../lib/findBus";
import { useFind } from "../hooks/useFind";
import { getWordWrap } from "../lib/editorPrefs";
import { bufferKey, dropBuffer, stashBuffer, takeBuffer } from "../lib/editorBuffers";
import { isExternalHref, onMarkdownLinkClick, renderMarkdown } from "../lib/mdHtml";
import { toastError } from "../lib/toast";
import ConfirmDialog from "./ConfirmDialog";
import Menu, { MenuEntry } from "./git/Menu";

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
}>(function FileEditor({ root, path, onDirtyChange }, ref) {
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
  // Binary payload (data URL) for image/pdf files.
  const [dataUrl, setDataUrl] = useState("");
  // Preview toggle for md/html/svg. previewContent holds sanitized HTML for
  // markdown, and the frame/image source (a data: URL) for html and svg.
  const [previewing, setPreviewing] = useState(false);
  const [previewContent, setPreviewContent] = useState("");
  // The rendered markdown itself, for Select all.
  const mdRef = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  // Guards the destructive revert behind a confirmation when there are edits.
  const [confirmRevert, setConfirmRevert] = useState(false);

  const vk = viewKind(path);

  // ⌘F / Edit ▸ Find, over whichever view is live in this tab.
  const findEngine = useMemo(() => cmFindEngine(() => viewRef.current), []);
  const { bar: findBar, onContentChange } = useFind({
    host: wrapRefEl,
    engine: findEngine,
    canReplace: true,
    rank: FindRank.editor,
    // Images, PDFs, media and the rendered preview have no text buffer to
    // search — ⌘F should reach past them, not open a bar over nothing.
    enabled: vk.kind === "text" && !previewing,
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
      const view = new EditorView({ state, parent: host });
      viewRef.current = view;
      if (stashed !== null) markDirtyRef.current(true);
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
  function showPreview() {
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
      { label: "Copy", hint: "⌘C", disabled: !selected, onClick: () => void copyText(selected, "Couldn't copy") },
      { label: "Select all", onClick: selectAllPreview },
      { kind: "separator" },
      { label: "Show source", onClick: () => setPreviewing(false) },
    );
    setMenu({ x: e.clientX, y: e.clientY, items });
  };

  const previewable = vk.kind === "text" && vk.preview !== null;

  return (
    <div className="file-editor-wrap" ref={wrapRefEl}>
      {findBar}
      <div className="file-editor-bar">
        <span className="file-editor-path">{path}{dirty ? " ●" : ""}</span>
        <span className="spacer" style={{ flex: 1 }} />
        {previewable && status === "ready" && (
          <div className="seg seg-mini">
            <button className={previewing ? "" : "on"} onClick={() => setPreviewing(false)}>Code</button>
            <button className={previewing ? "on" : ""} onClick={showPreview}>Preview</button>
          </div>
        )}
        {vk.kind === "text" && (
          <>
            <button className="file-editor-btn" title="Revert changes" aria-label="Revert changes"
              disabled={!dirty || status !== "ready"} onClick={() => setConfirmRevert(true)}>
              <RevertGlyph />
            </button>
            <button className="file-editor-btn" title="Save (⌘S)" aria-label="Save"
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
              <div className="file-preview-doc" onContextMenu={openPreviewMenu}>
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
