import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { EditorState, Transaction } from "@codemirror/state";
import {
  EditorView, drawSelection, dropCursor, keymap, placeholder as cmPlaceholder, tooltips,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { FileRoot } from "../api";
import { DocsIndex } from "../lib/docsIndex";
import { CrossRefs } from "../lib/links";
import { editorChromeTheme } from "../lib/cmTheme";
import { cmFindExtensions } from "../lib/cmFind";
import {
  docsCompletion, docsHighlight, docsMarkdown, docsNavFacet, livePreview,
} from "../lib/livePreview";
import { markdownTables } from "../lib/mdTable";
import { useLiveFacets } from "../hooks/useLiveFacets";
import { formatCommand, toggleInline } from "../lib/mdFormat";
import Menu from "./git/Menu";
import { caretToPointer, markdownMenuItems } from "./mdMenu";

/** Imperative access for callers that splice text in (attachments, find). */
export interface MarkdownEditorHandle {
  /** The live view, for the find engine. Null before mount / after unmount. */
  view: () => EditorView | null;
  /** The document as it stands — fresher than any React state mirroring it. */
  text: () => string;
  /** Replace the whole document; `caret` parks the cursor and takes focus. */
  setText: (text: string, caret?: number) => void;
}

// Document-feel chrome over the shared editor theme: prose in the UI sans, no
// code-editor furniture. Type scale and the box's own height are left to CSS —
// the same editor is a 120px card in the narrow pane and a full reading column
// when the issue is expanded.
const proseTheme = EditorView.theme({
  ".cm-content": {
    fontFamily: "var(--sans)",
    padding: "4px 6px 8px",
    caretColor: "var(--text)",
    // Bleed for live-preview block backgrounds; keep in step with the
    // horizontal padding above (see .lp-codeblock in styles.css).
    "--lp-gut": "6px",
  },
  // Zero so the selection layer's rectangles line up with those backgrounds
  // (CM's default 6px/2px would offset them).
  ".cm-line": { padding: "0" },
  ".cm-scroller": { fontFamily: "var(--sans)", lineHeight: "inherit" },
  ".cm-activeLine": { backgroundColor: "transparent" },
});

/**
 * A markdown editor with the Docs tab's Obsidian-style live preview: headings
 * render, syntax marks hide until the caret lands on them, images and
 * checkboxes draw as widgets, wikilinks resolve.
 *
 * Unlike DocsEditor (which owns a file and autosaves), this one edits a string
 * the caller holds: `value` seeds the document and follows external rewrites,
 * every keystroke comes back through `onChange`, and when the text should be
 * committed is entirely the caller's business.
 */
export default forwardRef<MarkdownEditorHandle, {
  className?: string;
  value: string;
  placeholder?: string;
  /** Where the file being edited lives — relative image paths resolve from it. */
  root: FileRoot;
  dir: string; // repo-relative directory
  path: string; // file name within `dir`
  index: DocsIndex | null; // wikilink resolution
  cross: CrossRefs | null; // typed wikilinks ([[AGE-14]], [[run:id]])
  onChange: (text: string) => void;
  onBlur?: (text: string) => void;
  onNavigate: (target: string, heading: string | null) => void;
  onTagClick: (tag: string) => void;
  /** Files pasted onto the editor, with where they were dropped in the text. */
  onPasteFiles?: (files: File[], at: { from: number; to: number; text: string }) => void;
}>(function MarkdownEditor({
  className, value, placeholder, root, dir, path, index, cross,
  onChange, onBlur, onNavigate, onTagClick, onPasteFiles,
}, ref) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  // Everything the extensions call out to, behind refs: callback identity
  // churns on every render of the pane, and a reconfigure per keystroke isn't
  // worth it. The view itself is built once.
  const live = useRef({ onChange, onBlur, onNavigate, onTagClick, onPasteFiles, root, dir, path });
  live.current = { onChange, onBlur, onNavigate, onTagClick, onPasteFiles, root, dir, path };

  // Index / cross-ref refreshes reach the decorations through compartments,
  // reconfigured only when the data the extensions read actually changed.
  const liveFacets = useLiveFacets(viewRef, index, cross);

  // The text most recently seen from (or handed back to) the caller: what the
  // `value` sync below compares against.
  const valueRef = useRef(value);

  useImperativeHandle(ref, () => ({
    view: () => viewRef.current,
    text: () => viewRef.current?.state.doc.toString() ?? valueRef.current,
    setText: (text: string, caret?: number) => {
      const view = viewRef.current;
      if (!view) return;
      const cur = view.state.doc.toString();
      valueRef.current = text;
      if (cur === text) {
        if (caret !== undefined) {
          view.dispatch({ selection: { anchor: Math.min(caret, text.length) } });
          view.focus();
        }
        return;
      }
      view.dispatch({
        changes: { from: 0, to: cur.length, insert: text },
        ...(caret !== undefined ? { selection: { anchor: Math.min(caret, text.length) } } : {}),
        // A rewrite from outside isn't the user's edit to take back: undoing an
        // attachment insert would drop the link and strand the file on disk.
        annotations: Transaction.addToHistory.of(false),
      });
      if (caret !== undefined) view.focus();
    },
  }));

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const state = EditorState.create({
      doc: valueRef.current,
      extensions: [
        history(),
        // drawSelection paints selection on a layer below in-flow content, so
        // the live-preview backgrounds have to live on ::before pseudo-elements
        // — see the note in DocsEditor and .md-live in styles.css.
        drawSelection(),
        dropCursor(),
        EditorView.lineWrapping,
        editorChromeTheme,
        docsHighlight,
        proseTheme,
        docsMarkdown(),
        livePreview,
        markdownTables,
        docsCompletion,
        // The editor can sit in a scrolling pane barely taller than itself
        // (the contracted issue detail), where an absolutely-positioned
        // completion list would be clipped by the box it belongs to.
        tooltips({ position: "fixed" }),
        cmFindExtensions,
        ...(placeholder ? [cmPlaceholder(placeholder)] : []),
        ...liveFacets(),
        docsNavFacet.of({
          onNavigate: (t, h) => live.current.onNavigate(t, h),
          onTagClick: (t) => live.current.onTagClick(t),
          get root() { return live.current.root; },
          get docsDir() { return live.current.dir; },
          get notePath() { return live.current.path; },
        }),
        EditorView.domEventHandlers({
          blur: (_e, v) => {
            live.current.onBlur?.(v.state.doc.toString());
            return false;
          },
          paste: (e, v) => {
            const files = [...(e.clipboardData?.files ?? [])].filter((f) => f.type.startsWith("image/"));
            if (files.length === 0 || !live.current.onPasteFiles) return false;
            e.preventDefault();
            const sel = v.state.selection.main;
            live.current.onPasteFiles(files, { from: sel.from, to: sel.to, text: v.state.doc.toString() });
            return true;
          },
        }),
        keymap.of([
          // preventDefault keeps WebKit's own rich-text commands off the
          // contenteditable, which would wrap the selection in <b>/<i> nodes
          // CodeMirror never asked for.
          { key: "Mod-b", preventDefault: true, run: formatCommand((s) => toggleInline(s, "bold")) },
          { key: "Mod-i", preventDefault: true, run: formatCommand((s) => toggleInline(s, "italic")) },
          ...defaultKeymap,
          ...historyKeymap,
        ]),
        EditorView.updateListener.of((u) => {
          if (!u.docChanged) return;
          const text = u.state.doc.toString();
          valueRef.current = text;
          live.current.onChange(text);
        }),
      ],
    });
    viewRef.current = new EditorView({ state, parent: host });
    return () => {
      viewRef.current?.destroy();
      viewRef.current = null;
    };
    // Built once: the document follows `value`, and everything else is behind
    // a ref or a compartment.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Follow a rewrite from outside (an attachment spliced in, a link removed).
  // Compared against the live document, so a keystroke that hasn't reached the
  // caller's state yet is never clobbered.
  useEffect(() => {
    const view = viewRef.current;
    if (!view || value === valueRef.current) return;
    const cur = view.state.doc.toString();
    valueRef.current = value;
    if (cur === value) return;
    view.dispatch({
      changes: { from: 0, to: cur.length, insert: value },
      annotations: Transaction.addToHistory.of(false),
    });
  }, [value]);

  const openMenu = (e: React.MouseEvent) => {
    const view = viewRef.current;
    if (!view) return;
    e.preventDefault();
    caretToPointer(view, e.clientX, e.clientY);
    setMenu({ x: e.clientX, y: e.clientY });
  };

  return (
    <>
      <div ref={hostRef} className={className} onContextMenu={openMenu} />
      {menu && viewRef.current && (
        <Menu x={menu.x} y={menu.y} items={markdownMenuItems(viewRef.current)} onClose={() => setMenu(null)} />
      )}
    </>
  );
});
