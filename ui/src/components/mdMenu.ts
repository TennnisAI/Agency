// The right-click menu shared by every live-preview markdown surface (the
// Docs note editor and the issue description): Obsidian-style formatting over
// a CodeMirror view, plus the clipboard items a context menu is expected to
// carry. The transforms themselves are pure and live in lib/mdFormat.

import { EditorState, TransactionSpec } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { clipboardText } from "../lib/clipboard";
import { toastError } from "../lib/toast";
import {
  blockState, clearFormatting, insertConstruct, setHeading,
  toggleInline, toggleList, toggleQuote,
} from "../lib/mdFormat";
import type { MenuEntry } from "./git/Menu";
import { shortcutLabel } from "../lib/platform";

/** Run a formatting transform against the view, then hand focus back. */
function apply(view: EditorView, f: (state: EditorState) => TransactionSpec | null) {
  return () => {
    const spec = f(view.state);
    if (spec) view.dispatch(spec);
    view.focus();
  };
}

async function copySelection(view: EditorView, andCut: boolean) {
  const sel = view.state.selection.main;
  if (sel.empty) return;
  try {
    await navigator.clipboard.writeText(view.state.sliceDoc(sel.from, sel.to));
    if (andCut) {
      view.dispatch({ changes: { from: sel.from, to: sel.to }, selection: { anchor: sel.from } });
    }
  } catch (e) {
    toastError(e, andCut ? "Couldn't cut" : "Couldn't copy");
  }
  view.focus();
}

async function pasteClipboard(view: EditorView) {
  try {
    // Not navigator.clipboard: on a multi-item clipboard the webview's own
    // reader can only see the first item (see lib/clipboard.ts).
    const text = await clipboardText();
    if (text) {
      const sel = view.state.selection.main;
      view.dispatch({
        changes: { from: sel.from, to: sel.to, insert: text },
        selection: { anchor: sel.from + text.length },
      });
    }
  } catch (e) {
    // Reading the clipboard can be refused by the webview; the key still works.
    toastError(e, `Couldn't paste (${shortcutLabel("⌘V")} still works)`);
  }
  view.focus();
}

/**
 * Build the menu for `view`, reflecting what the caret currently sits in
 * (checked headings, list kind, quote). Call it as the menu opens — the entries
 * capture the state they were built from.
 */
export function markdownMenuItems(view: EditorView): MenuEntry[] {
  const block = blockState(view.state);
  const hasSelection = !view.state.selection.main.empty;
  const heading = (level: number): MenuEntry => ({
    label: `Heading ${level}`,
    checked: block.heading === level,
    onClick: apply(view, (s) => setHeading(s, level)),
  });
  return [
    {
      kind: "submenu", label: "Format", items: [
        // No shortcut hints: ⌘B is the native menu's Toggle Sidebar
        // accelerator, and macOS gives the menu bar the key first.
        { label: "Bold", onClick: apply(view, (s) => toggleInline(s, "bold")) },
        { label: "Italic", onClick: apply(view, (s) => toggleInline(s, "italic")) },
        { label: "Strikethrough", onClick: apply(view, (s) => toggleInline(s, "strike")) },
        { label: "Highlight", onClick: apply(view, (s) => toggleInline(s, "highlight")) },
        { kind: "separator" },
        { label: "Code", onClick: apply(view, (s) => toggleInline(s, "code")) },
        { kind: "separator" },
        { label: "Clear formatting", onClick: apply(view, clearFormatting) },
      ],
    },
    {
      kind: "submenu", label: "Paragraph", items: [
        { label: "Bullet list", checked: block.list === "bullet", onClick: apply(view, (s) => toggleList(s, "bullet")) },
        { label: "Numbered list", checked: block.list === "ordered", onClick: apply(view, (s) => toggleList(s, "ordered")) },
        { label: "Task list", checked: block.list === "task", onClick: apply(view, (s) => toggleList(s, "task")) },
        { kind: "separator" },
        ...[1, 2, 3, 4, 5, 6].map(heading),
        { label: "Body", checked: block.heading === 0, onClick: apply(view, (s) => setHeading(s, 0)) },
        { kind: "separator" },
        { label: "Quote", checked: block.quote, onClick: apply(view, toggleQuote) },
      ],
    },
    {
      kind: "submenu", label: "Insert", items: [
        { label: "Wikilink", onClick: apply(view, (s) => insertConstruct(s, "wikilink")) },
        { label: "Link", onClick: apply(view, (s) => insertConstruct(s, "link")) },
        { kind: "separator" },
        { label: "Table", onClick: apply(view, (s) => insertConstruct(s, "table")) },
        { label: "Callout", onClick: apply(view, (s) => insertConstruct(s, "callout")) },
        { label: "Horizontal rule", onClick: apply(view, (s) => insertConstruct(s, "rule")) },
        { label: "Code block", onClick: apply(view, (s) => insertConstruct(s, "codeblock")) },
      ],
    },
    { kind: "separator" },
    { label: "Cut", hint: shortcutLabel("⌘X"), disabled: !hasSelection, onClick: () => void copySelection(view, true) },
    { label: "Copy", hint: shortcutLabel("⌘C"), disabled: !hasSelection, onClick: () => void copySelection(view, false) },
    { label: "Paste", hint: shortcutLabel("⌘V"), onClick: () => void pasteClipboard(view) },
    { kind: "separator" },
    {
      label: "Select all",
      hint: shortcutLabel("⌘A"),
      onClick: apply(view, (s) => ({ selection: { anchor: 0, head: s.doc.length } })),
    },
  ];
}

/**
 * Move the caret to where the pointer is before opening the menu, the way every
 * text editor does — formatting then targets what was clicked, not wherever the
 * selection happened to be. A right-click *inside* the selection keeps it.
 */
export function caretToPointer(view: EditorView, x: number, y: number) {
  const pos = view.posAtCoords({ x, y });
  const sel = view.state.selection.main;
  if (pos !== null && (pos < sel.from || pos > sel.to)) {
    view.dispatch({ selection: { anchor: pos } });
  }
  view.focus();
}
