import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, drawSelection, dropCursor, keymap } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { FileRoot, createDir, readFile, writeFile, writeFileBase64 } from "../api";
import { toastError } from "../lib/toast";
import { editorChromeTheme } from "../lib/cmTheme";
import { cmFindEngine, cmFindExtensions } from "../lib/cmFind";
import { FindRank } from "../lib/findBus";
import { useFind } from "../hooks/useFind";
import { DocsIndex } from "../lib/docsIndex";
import { CrossRefs } from "../lib/links";
import { docsCompletion, docsHighlight, docsMarkdown, docsNavFacet, livePreview, DocsNav } from "../lib/livePreview";
import { useLiveFacets } from "../hooks/useLiveFacets";
import { frontmatterEditor, requestAddProperty } from "../lib/fmEditor";
import { joinPath } from "../lib/filePath";
import { formatCommand, toggleInline } from "../lib/mdFormat";
import { minimalReplacement } from "../lib/textEdit";
import Menu from "./git/Menu";
import { caretToPointer, markdownMenuItems } from "./mdMenu";

export interface DocsEditorHandle {
  /** Scroll the first heading whose text matches (case-insensitive) into view. */
  scrollToHeading: (text: string) => void;
  /** Flush any pending autosave immediately. */
  flush: () => Promise<void>;
  /** Grow the properties card by one row (creating the block if absent). */
  addProperty: () => void;
  /** Type text in at the cursor, as if the user had. Backs the offer to link a
   * just-dropped attachment from the note being written. */
  insertAtCursor: (text: string) => void;
}

const AUTOSAVE_MS = 800;

// Document-feel chrome layered over the shared editor theme: prose in the UI
// sans, a centered measure, and no code-editor furniture (gutters etc. are
// simply not installed).
const docsEditorTheme = EditorView.theme({
  "&": { fontSize: "14.5px", height: "100%" },
  ".cm-content": {
    fontFamily: "-apple-system, system-ui, sans-serif",
    maxWidth: "720px",
    margin: "0 auto",
    padding: "24px 32px 120px",
    lineHeight: "1.65",
    caretColor: "var(--text)",
    // How far live-preview block backgrounds bleed past the text column, so
    // they cover the same span as the selection layer's rectangles. Keep in
    // step with the horizontal padding above (.lp-codeblock in styles.css).
    "--lp-gut": "32px",
  },
  // Zero, so the selection layer measures its rectangles from the content
  // edge — the fixed 6px/2px CM puts here would offset them from the block
  // backgrounds by exactly that much.
  ".cm-line": { padding: "0" },
  ".cm-scroller": { fontFamily: "-apple-system, system-ui, sans-serif" },
  ".cm-activeLine": { backgroundColor: "transparent" },
});

/**
 * The Docs tab's note editor. Markdown-only, autosaving (debounced, flushed on
 * Mod-s / note switch / unmount / window blur), and — while clean — following
 * external edits picked up by the corpus poll (`diskText`).
 */
export default forwardRef<DocsEditorHandle, {
  root: FileRoot;
  docsDir: string;
  path: string; // rel to docs dir
  diskText: string | undefined; // latest corpus copy of this note
  index: DocsIndex | null; // for wikilink resolution styling
  cross: CrossRefs | null; // for typed wikilinks ([[AGE-14]], [[run:id]])
  onSaved: () => void; // refresh the index after a write lands
  onNavigate: (target: string, heading: string | null) => void; // wikilink follow
  onTagClick: (tag: string) => void;
  onFilter: (key: string, value: string) => void; // properties card filter glyph
  sideOpen: boolean;
  onToggleSide: () => void;
  // Journal notes get prev/next-day navigation; null hides the buttons. Targets
  // are the nearest *existing* daily notes (navigation never creates files).
  daily?: { prev: string | null; next: string | null; onOpen: (path: string) => void } | null;
}>(function DocsEditor({ root, docsDir, path, diskText, index, cross, onSaved, onNavigate, onTagClick, onFilter, sideOpen, onToggleSide, daily }, ref) {
  const hostRef = useRef<HTMLDivElement>(null);
  // The whole column, not just the editor host: ⌘F resolution asks which
  // surface holds focus, and the find bar has to count as part of this one.
  const colRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [errorMsg, setErrorMsg] = useState("");
  const [saveState, setSaveState] = useState<"clean" | "dirty" | "saving">("clean");
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const timerRef = useRef<number | null>(null);
  // The text most recently loaded from or written to disk, for clean checks.
  const savedTextRef = useRef("");
  const saveStateRef = useRef(saveState);
  saveStateRef.current = saveState;
  // The corpus copy the disk-follow effect has already reconciled against, and
  // a live view of the current one for the load path (which runs inside an
  // async callback, where the captured prop is a render behind).
  const seenDiskRef = useRef<string | undefined>(undefined);
  const diskTextRef = useRef(diskText);
  diskTextRef.current = diskText;

  // ⌘F / Edit ▸ Find. One engine per editor instance (there is one per open
  // tab), reading whichever view is live through the ref.
  const findEngine = useMemo(() => cmFindEngine(() => viewRef.current), []);
  const { bar: findBar, onContentChange } = useFind({
    host: colRef,
    engine: findEngine,
    canReplace: true,
    rank: FindRank.editor,
  });
  const onFindContentChange = useRef(onContentChange);
  onFindContentChange.current = onContentChange;

  const repoRel = joinPath(docsDir, path);
  const onSavedRef = useRef(onSaved);
  onSavedRef.current = onSaved;
  const navRef = useRef<Pick<DocsNav, "onNavigate" | "onTagClick" | "onFilter">>({ onNavigate, onTagClick, onFilter });
  navRef.current = { onNavigate, onTagClick, onFilter };
  // Index / cross-ref refreshes reach the decorations through compartments,
  // reconfigured only when the data the extensions read actually changed.
  const liveFacets = useLiveFacets(viewRef, index, cross);

  // Bound inside the load effect so it always writes to the note the live view
  // belongs to. (Binding on render would point a pre-switch flush at the NEXT
  // note's path — a save-to-wrong-file bug.)
  const save = useRef(async () => {});

  // Paste an image: save the bytes to docs/assets/ and insert a markdown link
  // at the cursor. Names are timestamped; the no-clobber write gets a numeric
  // suffix retry for burst pastes within the same second.
  const pasteImageRef = useRef(async (_file: File, _view: EditorView) => {});
  pasteImageRef.current = async (file: File, view: EditorView) => {
    try {
      const buf = new Uint8Array(await file.arrayBuffer());
      let bin = "";
      const CHUNK = 0x8000;
      for (let i = 0; i < buf.length; i += CHUNK) {
        bin += String.fromCharCode(...buf.subarray(i, i + CHUNK));
      }
      const b64 = btoa(bin);
      const ext = ({ "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif", "image/webp": "webp" })[file.type] ?? "png";
      const d = new Date();
      const pad = (n: number) => String(n).padStart(2, "0");
      const stamp = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
      await createDir(root, joinPath(docsDir, "assets")).catch(() => { /* already exists */ });
      let name = `pasted-${stamp}.${ext}`;
      for (let attempt = 2; attempt <= 5; attempt++) {
        try {
          await writeFileBase64(root, joinPath(docsDir, `assets/${name}`), b64);
          break;
        } catch (e) {
          if (attempt === 5) throw e;
          name = `pasted-${stamp}-${attempt}.${ext}`;
        }
      }
      const { from, to } = view.state.selection.main;
      view.dispatch({
        changes: { from, to, insert: `![](assets/${name})` },
        selection: { anchor: from + `![](assets/${name})`.length },
      });
    } catch (e) {
      toastError(e, "Couldn't paste image");
    }
  };

  const flush = async () => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    await save.current();
  };
  const flushRef = useRef(flush);
  flushRef.current = flush;

  useImperativeHandle(ref, () => ({
    flush: () => flushRef.current(),
    addProperty: () => {
      const view = viewRef.current;
      if (view) requestAddProperty(view);
    },
    insertAtCursor: (text: string) => {
      const view = viewRef.current;
      if (!view) return;
      const { from, to } = view.state.selection.main;
      view.dispatch({
        changes: { from, to, insert: text },
        selection: { anchor: from + text.length },
      });
      view.focus();
    },
    scrollToHeading: (text: string) => {
      const view = viewRef.current;
      if (!view) return;
      const want = text.trim().toLowerCase();
      const doc = view.state.doc;
      for (let i = 1; i <= doc.lines; i++) {
        const line = doc.line(i);
        const m = /^#{1,6}\s+(.+)$/.exec(line.text);
        if (m && m[1].trim().toLowerCase() === want) {
          view.dispatch({
            selection: { anchor: line.from },
            effects: EditorView.scrollIntoView(line.from, { y: "start", yMargin: 12 }),
          });
          view.focus();
          return;
        }
      }
    },
  }));

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setSaveState("clean");
    setErrorMsg("");

    readFile(root, repoRel).then((fc) => {
      if (cancelled) return;
      if (fc.tooLarge || fc.binary) {
        setStatus("error");
        setErrorMsg(fc.tooLarge ? "File too large to open." : "Not a text file.");
        return;
      }
      const host = hostRef.current;
      if (!host) return;
      setStatus("ready");
      savedTextRef.current = fc.text;
      // The read above is fresher than anything the poll has published, so
      // whatever the corpus holds right now counts as already reconciled.
      seenDiskRef.current = diskTextRef.current;
      save.current = async () => {
        const view = viewRef.current;
        if (!view || saveStateRef.current === "clean") return;
        const text = view.state.doc.toString();
        if (text === savedTextRef.current) {
          setSaveState("clean");
          return;
        }
        setSaveState("saving");
        try {
          await writeFile(root, repoRel, text);
          savedTextRef.current = text;
          // Only mark clean if no further edits arrived while the write was
          // in flight; the update listener flips state back to dirty if so.
          if (viewRef.current?.state.doc.toString() === text) setSaveState("clean");
          onSavedRef.current();
        } catch (e) {
          setSaveState("dirty");
          setErrorMsg(String(e));
          setStatus("error");
        }
      };
      viewRef.current?.destroy();
      const state = EditorState.create({
        doc: fc.text,
        extensions: [
          history(),
          // drawSelection paints selection on a layer at z-index -1, below
          // in-flow content — so decoration backgrounds (.lp-code, .lp-codeblock,
          // .lp-callout) MUST live on ::before pseudo-elements at z-index -3
          // (see styles.css) or they occlude the selection. The native
          // ::selection is no alternative: WebKit paints it behind element
          // backgrounds entirely (verified in Safari).
          drawSelection(),
          dropCursor(),
          EditorView.lineWrapping,
          editorChromeTheme,
          docsHighlight,
          docsEditorTheme,
          docsMarkdown(),
          livePreview,
          docsCompletion,
          cmFindExtensions,
          ...liveFacets(),
          // A stable nav facade reading live refs, so callback identity churn
          // never forces a reconfigure.
          docsNavFacet.of({
            onNavigate: (t, h) => navRef.current.onNavigate(t, h),
            onTagClick: (t) => navRef.current.onTagClick(t),
            onFilter: (k, v) => navRef.current.onFilter?.(k, v),
            root,
            docsDir,
            notePath: path,
          }),
          frontmatterEditor,
          EditorView.domEventHandlers({
            paste: (e, v) => {
              const file = [...(e.clipboardData?.files ?? [])].find((f) => f.type.startsWith("image/"));
              if (!file) return false;
              e.preventDefault();
              void pasteImageRef.current(file, v);
              return true;
            },
          }),
          keymap.of([
            { key: "Mod-s", preventDefault: true, run: () => { void flushRef.current(); return true; } },
            // Bold/italic keys. preventDefault also keeps WebKit's own
            // rich-text editing commands off the contenteditable, which would
            // otherwise try to wrap the selection in <b>/<i> nodes CodeMirror
            // never asked for. (⌘B usually never arrives: the native menu's
            // Toggle Sidebar accelerator claims it first.)
            { key: "Mod-b", preventDefault: true, run: formatCommand((s) => toggleInline(s, "bold")) },
            { key: "Mod-i", preventDefault: true, run: formatCommand((s) => toggleInline(s, "italic")) },
            ...defaultKeymap,
            ...historyKeymap,
          ]),
          EditorView.updateListener.of((u) => {
            if (!u.docChanged) return;
            onFindContentChange.current();
            setSaveState((s) => (s === "saving" ? s : "dirty"));
            if (timerRef.current !== null) window.clearTimeout(timerRef.current);
            timerRef.current = window.setTimeout(() => {
              timerRef.current = null;
              void save.current();
            }, AUTOSAVE_MS);
          }),
        ],
      });
      viewRef.current = new EditorView({ state, parent: host });
    }).catch((e) => {
      if (cancelled) return;
      setErrorMsg(String(e));
      setStatus("error");
    });

    return () => {
      cancelled = true;
      // Flush pending edits before the note (or the whole tab) goes away.
      void flushRef.current();
      viewRef.current?.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [root.kind, root.id, repoRel]);

  // Flush when the window loses focus, so edits land before e.g. an agent or
  // external editor touches the same file.
  useEffect(() => {
    const onBlur = () => void flushRef.current();
    window.addEventListener("blur", onBlur);
    return () => window.removeEventListener("blur", onBlur);
  }, []);

  // ── Right-click menu: markdown formatting, Obsidian-style ─────────────────
  // Shared with the issue description (components/mdMenu).

  const openMenu = (e: React.MouseEvent) => {
    const view = viewRef.current;
    if (!view) return;
    e.preventDefault();
    caretToPointer(view, e.clientX, e.clientY);
    setMenu({ x: e.clientX, y: e.clientY });
  };

  // While clean, follow external edits surfaced by the corpus poll. A dirty
  // buffer wins (last-write-wins: our autosave lands within a second anyway).
  //
  // Only a corpus copy we haven't seen before is worth reconciling, and that
  // guard is load-bearing: this effect also re-runs when `saveState` flips, and
  // at the moment our own save flips it to clean the corpus still holds the
  // PRE-save copy — the refresh that save kicks off lands a beat later. Acting
  // on that stale copy rewrote the buffer with the previous text, which mapped
  // the cursor and the scroll anchor to position 0; the note lurched to the top
  // about a second after every edit, then lurched again when the real copy
  // arrived (AGE-89).
  useEffect(() => {
    const view = viewRef.current;
    if (!view || status !== "ready") return;
    if (diskText === undefined || diskText === seenDiskRef.current) return;
    seenDiskRef.current = diskText;
    if (saveState !== "clean" || diskText === savedTextRef.current) return;
    // Smallest change that gets there, so the reader's cursor and scroll
    // position map through it instead of collapsing to the top of the note.
    const edit = minimalReplacement(view.state.doc.toString(), diskText);
    savedTextRef.current = diskText;
    if (!edit) return;
    if (import.meta.env.DEV) console.debug(`[docs] external edit applied: ${path}`);
    view.dispatch({ changes: edit });
    // The dispatch flips saveState to dirty via the update listener — undo that,
    // this is disk state, not an edit.
    setSaveState("clean");
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, [diskText, status, saveState, path]);

  return (
    <div className="docs-editor-col" ref={colRef}>
      {findBar}
      <div className="docs-editor-head">
        {daily && (
          <span className="daily-nav">
            <button
              className="file-editor-btn"
              title="Previous daily note"
              disabled={!daily.prev}
              onClick={() => daily.prev && daily.onOpen(daily.prev)}
            >‹</button>
            <button
              className="file-editor-btn"
              title="Next daily note"
              disabled={!daily.next}
              onClick={() => daily.next && daily.onOpen(daily.next)}
            >›</button>
          </span>
        )}
        <span className="docs-editor-path" title={repoRel}>{path}</span>
        <span className="spacer" style={{ flex: 1 }} />
        <span className={`docs-save-state ${saveState}`}>
          {saveState === "clean" ? "saved" : saveState === "saving" ? "saving…" : "editing…"}
        </span>
        <button
          className={`file-editor-btn${sideOpen ? " on" : ""}`}
          title={sideOpen ? "Hide side panel" : "Show side panel (outline, backlinks, agents)"}
          onClick={onToggleSide}
        >
          <PanelGlyph />
        </button>
      </div>
      {status === "loading" && <div className="diff-empty">loading…</div>}
      {status === "error" && <div className="git-error">{errorMsg}</div>}
      <div ref={hostRef} className="docs-editor-host md-live" onContextMenu={openMenu}
        style={{ display: status === "ready" ? "block" : "none" }} />
      {menu && viewRef.current && (
        <Menu x={menu.x} y={menu.y} items={markdownMenuItems(viewRef.current)} onClose={() => setMenu(null)} />
      )}
    </div>
  );
});

// Right-panel glyph: a frame with the right third divided off.
function PanelGlyph() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth={1.9} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <line x1="15" y1="4" x2="15" y2="20" />
    </svg>
  );
}
