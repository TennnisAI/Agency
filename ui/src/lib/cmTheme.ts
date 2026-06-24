import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

// The file editor was hardcoded to `oneDark`, so it looked alien on every other
// theme (and unreadable on the light Paperback). These styles reference the
// app's CSS custom properties directly, so the editor matches the active theme
// and tracks live theme switches with no remount — `var(--x)` resolves against
// :root each repaint.

/** Editor chrome (background, gutters, cursor, selection) drawn from theme vars. */
export const editorChromeTheme: Extension = EditorView.theme({
  "&": { color: "var(--text)", backgroundColor: "var(--base)", fontSize: "13px" },
  ".cm-content": { caretColor: "var(--text)", fontFamily: "var(--mono)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--text)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
    backgroundColor: "var(--s1)",
  },
  ".cm-panels": { backgroundColor: "var(--mantle)", color: "var(--text)" },
  ".cm-gutters": { backgroundColor: "var(--mantle)", color: "var(--o0)", border: "none" },
  ".cm-activeLineGutter": { backgroundColor: "var(--s0)", color: "var(--sub0)" },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--s0) 45%, transparent)" },
  ".cm-selectionMatch": { backgroundColor: "var(--s0)" },
  ".cm-foldPlaceholder": { backgroundColor: "var(--s0)", color: "var(--sub0)", border: "none" },
  ".cm-scroller": { fontFamily: "var(--mono)" },
});

/** Syntax token colors mapped from theme vars (mirrors the diff highlighter). */
export const editorHighlight: Extension = syntaxHighlighting(
  HighlightStyle.define([
    { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--o1)", fontStyle: "italic" },
    { tag: [t.string, t.special(t.string), t.character], color: "var(--green)" },
    { tag: [t.regexp, t.escape], color: "var(--teal)" },
    { tag: [t.number, t.bool, t.null, t.atom], color: "var(--peach)" },
    { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword, t.definitionKeyword], color: "var(--mauve)" },
    { tag: [t.operator, t.punctuation, t.separator, t.bracket, t.brace], color: "var(--sub0)" },
    { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--blue)" },
    { tag: [t.typeName, t.className, t.namespace, t.standard(t.name)], color: "var(--yellow)" },
    { tag: [t.propertyName, t.attributeName], color: "var(--blue)" },
    { tag: [t.variableName], color: "var(--text)" },
    { tag: [t.tagName], color: "var(--blue)" },
    { tag: [t.constant(t.variableName), t.constant(t.name)], color: "var(--peach)" },
    { tag: [t.heading], color: "var(--blue)", fontWeight: "bold" },
    { tag: [t.link, t.url], color: "var(--teal)", textDecoration: "underline" },
    { tag: [t.emphasis], fontStyle: "italic" },
    { tag: [t.strong], fontWeight: "bold" },
    { tag: [t.invalid], color: "var(--red)" },
  ]),
);
