import { EditorState, Extension, Facet, Range, RangeSet } from "@codemirror/state";
import {
  Decoration, DecorationSet, EditorView, ViewPlugin, ViewUpdate, WidgetType,
} from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import type { MarkdownConfig, InlineContext } from "@lezer/markdown";
import { autocompletion, CompletionContext, CompletionResult } from "@codemirror/autocomplete";
import { openUrl } from "@tauri-apps/plugin-opener";
import { FileRoot, readFileBase64 } from "../api";
import { toastError } from "./toast";
import { DocsIndex, parseFrontmatter, stripExt } from "./docsIndex";
import { CrossRefs, issueCompletionOptions, wikilinkView } from "./links";
import { languageForFence } from "./cmLanguage";
import { joinPath } from "./filePath";

// Obsidian-style live preview for the Docs tab: one CodeMirror pane where
// markdown renders inline (headings sized, marks hidden, checkboxes clickable)
// and raw syntax is revealed on the line(s) the selection touches.

/** The docs index, for wikilink resolution. Reconfigured via a Compartment. */
export const docsIndexFacet = Facet.define<DocsIndex | null, DocsIndex | null>({
  combine: (v) => v[0] ?? null,
});

/** Cross-project issue/run refs, for typed wikilinks ([[AGE-14]], [[run:id]]). */
export const crossRefsFacet = Facet.define<CrossRefs | null, CrossRefs | null>({
  combine: (v) => v[0] ?? null,
});

export interface DocsNav {
  /** Follow a wikilink target (raw text between the brackets, sans alias). */
  onNavigate: (target: string, heading: string | null) => void;
  /** A #tag was clicked. */
  onTagClick: (tag: string) => void;
  /** Filter the docs search by a frontmatter property (properties card). */
  onFilter?: (key: string, value: string) => void;
  /** Where this note lives, for resolving relative image paths. */
  root: FileRoot;
  docsDir: string;
  notePath: string; // rel to docs dir
}

export const docsNavFacet = Facet.define<DocsNav | null, DocsNav | null>({
  combine: (v) => v[0] ?? null,
});

// ── Wikilink syntax: [[target#heading|alias]] as real parse nodes ────────────

const wikilinkExtension: MarkdownConfig = {
  defineNodes: [
    { name: "Wikilink" },
    { name: "WikilinkMark" },
  ],
  parseInline: [{
    name: "Wikilink",
    parse(cx: InlineContext, next: number, pos: number): number {
      if (next !== 91 /* [ */ || cx.char(pos + 1) !== 91) return -1;
      let end = -1;
      for (let i = pos + 2; i < cx.end - 1; i++) {
        const ch = cx.char(i);
        if (ch === 10 /* \n */) return -1;
        if (ch === 93 /* ] */ && cx.char(i + 1) === 93) { end = i; break; }
      }
      if (end < 0 || end === pos + 2) return -1;
      return cx.addElement(cx.elt("Wikilink", pos, end + 2, [
        cx.elt("WikilinkMark", pos, pos + 2),
        cx.elt("WikilinkMark", end, end + 2),
      ]));
    },
    before: "Link",
  }],
};

// ── Highlight syntax: ==marked text== (not GFM; Obsidian's, and ours) ────────

const highlightExtension: MarkdownConfig = {
  defineNodes: [
    { name: "Highlight" },
    { name: "HighlightMark" },
  ],
  parseInline: [{
    name: "Highlight",
    parse(cx: InlineContext, next: number, pos: number): number {
      if (next !== 61 /* = */ || cx.char(pos + 1) !== 61) return -1;
      let end = -1;
      for (let i = pos + 2; i < cx.end - 1; i++) {
        const ch = cx.char(i);
        if (ch === 10 /* \n */) return -1;
        if (ch === 61 && cx.char(i + 1) === 61) { end = i; break; }
      }
      if (end < 0 || end === pos + 2) return -1;
      return cx.addElement(cx.elt("Highlight", pos, end + 2, [
        cx.elt("HighlightMark", pos, pos + 2),
        cx.elt("HighlightMark", end, end + 2),
      ]));
    },
    before: "Emphasis",
  }],
};

/** Markdown language for docs notes: GFM base + wikilinks + fenced-code langs.
 *  Fenced blocks resolve through the same registry the file editor uses, so
 *  ```go and ```sql color in a note exactly as they do in a source file. */
export const docsMarkdown = () =>
  markdown({
    base: markdownLanguage,
    codeLanguages: languageForFence,
    extensions: [wikilinkExtension, highlightExtension],
  });

/**
 * Token colors for the docs editor. Mirrors cmTheme's editorHighlight for code
 * inside fenced blocks, but keeps prose constructs (headings, emphasis) in the
 * text color — live preview styles those via .lp-* classes, and the code
 * editor's blue headings read wrong in a document context.
 */
export const docsHighlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--o1)", fontStyle: "italic" },
    { tag: [t.string, t.special(t.string), t.character], color: "var(--green)" },
    { tag: [t.regexp, t.escape], color: "var(--teal)" },
    { tag: [t.number, t.bool, t.null, t.atom], color: "var(--peach)" },
    { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword, t.definitionKeyword], color: "var(--mauve)" },
    { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--blue)" },
    { tag: [t.typeName, t.className, t.namespace, t.standard(t.name)], color: "var(--yellow)" },
    { tag: [t.propertyName, t.attributeName], color: "var(--blue)" },
    { tag: [t.tagName], color: "var(--blue)" },
    { tag: [t.constant(t.variableName), t.constant(t.name)], color: "var(--peach)" },
    { tag: [t.heading], color: "var(--text)" },
    { tag: [t.link, t.url], color: "var(--teal)" },
    { tag: [t.invalid], color: "var(--red)" },
  ]),
);

// ── Widgets ──────────────────────────────────────────────────────────────────

// CodeMirror drops every DOM event that starts inside a widget unless the
// widget opts back in (`eventBelongsToEditor`), and the default is to drop
// them. So a click on a rendered bullet, image, rule or callout title in an
// issue description reached the editor not at all: no focus, no caret, and
// the letters typed next were read as the issue board's single-key shortcuts
// and swallowed into quick-add on the left (AGE-160). A widget that only
// draws markdown lets the click through and leaves the caret to CodeMirror;
// one that owns its click (the checkbox below, the code-block header, the
// table in mdTable.ts) keeps the default and takes focus itself.

class CheckboxWidget extends WidgetType {
  constructor(readonly checked: boolean) { super(); }
  eq(other: CheckboxWidget) { return other.checked === this.checked; }
  toDOM(view: EditorView) {
    const box = document.createElement("span");
    box.className = `lp-checkbox${this.checked ? " checked" : ""}`;
    box.setAttribute("role", "checkbox");
    box.setAttribute("aria-checked", String(this.checked));
    if (this.checked) {
      const ns = "http://www.w3.org/2000/svg";
      const svg = document.createElementNS(ns, "svg");
      for (const [k, v] of [["width", "10"], ["height", "10"], ["viewBox", "0 0 24 24"],
        ["fill", "none"], ["stroke", "currentColor"], ["stroke-width", "3.4"],
        ["stroke-linecap", "round"], ["stroke-linejoin", "round"], ["aria-hidden", "true"]]) {
        svg.setAttribute(k, v);
      }
      const check = document.createElementNS(ns, "polyline");
      check.setAttribute("points", "20 6 9 17 4 12");
      svg.appendChild(check);
      box.appendChild(svg);
    }
    box.addEventListener("mousedown", (e) => {
      e.preventDefault();
      const pos = view.posAtDOM(box);
      // The widget replaces the 3-char "[ ]"/"[x]" TaskMarker at `pos`.
      const cur = view.state.doc.sliceString(pos, pos + 3);
      if (/^\[[ xX]\]$/.test(cur)) {
        view.dispatch({ changes: { from: pos, to: pos + 3, insert: this.checked ? "[ ]" : "[x]" } });
      }
      // The preventDefault above keeps the caret where it was, but it also
      // means the click never focused the editor. Focus last (focusing
      // rebuilds the decorations, and `box` is gone once it has), and without
      // moving the caret: the box the user clicked in is the one that takes
      // the typing, and the line under the pointer stays rendered.
      view.focus();
    });
    return box;
  }
}

// ── Inline images: relative paths load through the backend as data URLs ──────

/** Collapse "." / ".." segments; returns null when the path escapes the root. */
export function normalizeRel(path: string): string | null {
  const out: string[] = [];
  for (const seg of path.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      if (out.length === 0) return null;
      out.pop();
    } else {
      out.push(seg);
    }
  }
  return out.join("/");
}

// data-URL cache, keyed by root+path. Failed loads cache as "" so a missing
// image doesn't re-fetch on every decoration rebuild.
const imageCache = new Map<string, Promise<string>>();

function loadImage(nav: DocsNav, src: string): Promise<string> {
  const noteDir = nav.notePath.includes("/") ? nav.notePath.slice(0, nav.notePath.lastIndexOf("/")) : "";
  // Note-relative first (standard markdown), docs-root-relative as fallback
  // (what image paste writes: "assets/…" from anywhere in the tree).
  const candidates = [
    normalizeRel(`${noteDir}/${src}`),
    normalizeRel(src),
  ].filter((p): p is string => p !== null && p !== "");
  const key = `${nav.root.kind}:${nav.root.id}:${nav.docsDir}:${candidates.join("|")}`;
  let cached = imageCache.get(key);
  if (!cached) {
    cached = (async () => {
      for (const rel of candidates) {
        try {
          // joinPath: a workspace vault has docsDir "" — a bare `${dir}/${rel}`
          // would produce a leading slash the path jail rejects as absolute.
          const bc = await readFileBase64(nav.root, joinPath(nav.docsDir, rel));
          if (!bc.tooLarge && bc.mime.startsWith("image/")) {
            return `data:${bc.mime};base64,${bc.b64}`;
          }
        } catch {
          /* try the next candidate */
        }
      }
      return "";
    })();
    imageCache.set(key, cached);
  }
  return cached;
}

class ImageWidget extends WidgetType {
  constructor(readonly src: string, readonly alt: string, readonly nav: DocsNav) { super(); }
  eq(other: ImageWidget) { return other.src === this.src && other.nav.notePath === this.nav.notePath; }
  ignoreEvent() { return false; } // draws only; the click is the editor's
  get estimatedHeight() { return 200; }
  toDOM(view: EditorView) {
    const wrap = document.createElement("span");
    wrap.className = "lp-img-wrap loading";
    wrap.textContent = this.alt || this.src;
    void loadImage(this.nav, this.src).then((url) => {
      if (!wrap.isConnected && !url) return;
      if (!url) {
        wrap.className = "lp-img-wrap missing";
        return;
      }
      const img = document.createElement("img");
      img.className = "lp-img";
      img.src = url;
      img.alt = this.alt;
      img.onload = () => view.requestMeasure();
      wrap.textContent = "";
      wrap.className = "lp-img-wrap";
      wrap.appendChild(img);
    });
    return wrap;
  }
}

// ── Callouts: > [!note] Title ────────────────────────────────────────────────

const CALLOUT_TYPES: Record<string, string> = {
  note: "note", info: "note", abstract: "note", summary: "note",
  tip: "tip", hint: "tip", important: "tip", success: "tip", check: "tip",
  warning: "warning", caution: "warning", question: "warning", todo: "warning",
  danger: "danger", error: "danger", failure: "danger", bug: "danger",
};

class CalloutTitleWidget extends WidgetType {
  constructor(readonly kind: string, readonly label: string) { super(); }
  eq(other: CalloutTitleWidget) { return other.kind === this.kind && other.label === this.label; }
  ignoreEvent() { return false; } // draws only; the click is the editor's
  toDOM() {
    const el = document.createElement("span");
    el.className = `lp-callout-title lp-callout-title-${this.kind}`;
    el.textContent = `◆ ${this.label}`;
    return el;
  }
}

// ── Fenced code: the fences become the block's chrome ────────────────────────

/** An opening fence line: indent, the ``` (or ~~~) run, then the info string. */
const FENCE_OPEN_RE = /^(\s*)(`{3,}|~{3,})(.*)$/;
/** A closing fence line: the run and nothing else. */
const FENCE_CLOSE_RE = /^\s*(`{3,}|~{3,})\s*$/;

/** The body of the fenced block containing `pos`, fences excluded — what the
 *  copy button puts on the clipboard. Null when `pos` is outside a block. */
export function fencedCodeAt(state: EditorState, pos: number): string | null {
  const inner = syntaxTree(state).resolveInner(pos, 1);
  let node: typeof inner | null = inner;
  while (node && node.name !== "FencedCode") node = node.parent;
  if (!node) return null;
  const doc = state.doc;
  const first = doc.lineAt(node.from);
  const last = doc.lineAt(node.to);
  const from = Math.min(first.to + 1, doc.length);
  // An unterminated block runs to the end of the node; a closed one stops
  // short of its closing fence line.
  const to = last.number > first.number && FENCE_CLOSE_RE.test(last.text)
    ? Math.max(from, last.from - 1)
    : node.to;
  return doc.sliceString(from, to);
}

function iconSvg(kind: "copy" | "check"): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  for (const [k, v] of [["width", "12"], ["height", "12"], ["viewBox", "0 0 24 24"],
    ["fill", "none"], ["stroke", "currentColor"], ["stroke-width", "2.2"],
    ["stroke-linecap", "round"], ["stroke-linejoin", "round"], ["aria-hidden", "true"]]) {
    svg.setAttribute(k, v);
  }
  if (kind === "check") {
    const check = document.createElementNS(ns, "polyline");
    check.setAttribute("points", "20 6 9 17 4 12");
    svg.appendChild(check);
  } else {
    const sheet = document.createElementNS(ns, "rect");
    for (const [k, v] of [["x", "9"], ["y", "9"], ["width", "12"], ["height", "12"], ["rx", "2.5"]]) {
      sheet.setAttribute(k, v);
    }
    const behind = document.createElementNS(ns, "path");
    behind.setAttribute("d", "M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1");
    svg.append(sheet, behind);
  }
  return svg;
}

/**
 * Replaces the opening fence line: the language on the left, a copy button on
 * the right. The block's own text is read from the document at click time
 * (through the widget's DOM position), so typing inside a code block doesn't
 * churn the widget.
 */
class CodeHeaderWidget extends WidgetType {
  constructor(readonly lang: string, readonly hasCode: boolean) { super(); }
  eq(other: CodeHeaderWidget) {
    return other.lang === this.lang && other.hasCode === this.hasCode;
  }
  ignoreEvent() { return true; } // the strip handles its own clicks
  toDOM(view: EditorView) {
    const head = document.createElement("span");
    head.className = "lp-cb-head";
    head.setAttribute("contenteditable", "false");
    // Clicking the strip (but not the button) parks the caret on the fence
    // line, which reveals it — that's how the language gets edited.
    head.addEventListener("mousedown", (e) => {
      if ((e.target as HTMLElement | null)?.closest(".lp-cb-copy")) return;
      e.preventDefault();
      view.dispatch({ selection: { anchor: view.posAtDOM(head) } });
      view.focus();
    });

    const lang = document.createElement("span");
    lang.className = "lp-cb-lang";
    lang.textContent = this.lang;
    head.appendChild(lang);

    if (!this.hasCode) return head;

    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "lp-cb-copy";
    btn.title = "Copy code";
    btn.setAttribute("aria-label", "Copy code");
    btn.tabIndex = -1; // a mouse target; Tab still belongs to the document
    btn.appendChild(iconSvg("copy"));
    let timer = 0;
    btn.addEventListener("mousedown", (e) => e.preventDefault()); // keep the caret put
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      const code = fencedCodeAt(view.state, view.posAtDOM(head));
      if (code === null) return;
      void navigator.clipboard.writeText(code).then(() => {
        btn.classList.add("done");
        btn.replaceChildren(iconSvg("check"));
        window.clearTimeout(timer);
        timer = window.setTimeout(() => {
          btn.classList.remove("done");
          btn.replaceChildren(iconSvg("copy"));
        }, 1400);
      }).catch((err) => toastError(err, "Couldn't copy to clipboard"));
    });
    head.appendChild(btn);
    return head;
  }
}

class BulletWidget extends WidgetType {
  eq() { return true; }
  ignoreEvent() { return false; } // draws only; the click is the editor's
  toDOM() {
    const el = document.createElement("span");
    el.className = "lp-bullet";
    el.textContent = "•";
    return el;
  }
}

class HRWidget extends WidgetType {
  eq() { return true; }
  ignoreEvent() { return false; } // draws only; the click is the editor's
  toDOM() {
    const el = document.createElement("span");
    el.className = "lp-hr";
    return el;
  }
}


// ── Decoration building ──────────────────────────────────────────────────────

/** A #tag and the boundary character before it. Exported because table
 *  cells are drawn by a widget (mdTable.ts) and re-run it over their text. */
export const TAG_RE = /(^|[\s(])#([A-Za-z0-9_][A-Za-z0-9_/-]*)/g;

/** Line numbers touched by any selection range. */
function activeLines(view: EditorView): Set<number> {
  const out = new Set<number>();
  for (const r of view.state.selection.ranges) {
    const a = view.state.doc.lineAt(r.from).number;
    const b = view.state.doc.lineAt(r.to).number;
    for (let n = a; n <= b; n++) out.add(n);
  }
  return out;
}

const activeKey = (s: Set<number>) => [...s].sort((a, b) => a - b).join(",");

const markDim = Decoration.mark({ class: "lp-mark" });
const hide = Decoration.replace({});

class LivePreviewPlugin {
  decorations: DecorationSet;
  atomic: DecorationSet;
  key: string;

  constructor(view: EditorView) {
    this.key = activeKey(activeLines(view));
    [this.decorations, this.atomic] = this.safeBuild(view);
  }

  update(u: ViewUpdate) {
    const key = activeKey(activeLines(u.view));
    const indexChanged =
      u.startState.facet(docsIndexFacet) !== u.state.facet(docsIndexFacet) ||
      u.startState.facet(crossRefsFacet) !== u.state.facet(crossRefsFacet);
    // focusChanged matters because reveal is focus-gated (see build).
    if (u.docChanged || u.viewportChanged || indexChanged || u.focusChanged || key !== this.key) {
      this.key = key;
      [this.decorations, this.atomic] = this.safeBuild(u.view);
    }
  }

  // A decoration bug on one odd document must degrade to plain markdown, not
  // surface as an app-level error toast on every keystroke.
  safeBuild(view: EditorView): [DecorationSet, DecorationSet] {
    try {
      return this.build(view);
    } catch (e) {
      console.error("livePreview decoration build failed:", e);
      return [Decoration.none, RangeSet.empty];
    }
  }

  build(view: EditorView): [DecorationSet, DecorationSet] {
    const decos: Range<Decoration>[] = [];
    const atomics: Range<Decoration>[] = [];
    // Reveal is focus-gated: an unfocused editor has no visible cursor, so
    // nothing should show raw syntax. Without this, a freshly opened note
    // reveals whatever line 1 holds (the cursor parks at position 0 on
    // mount) — most visibly the frontmatter block, which rendered as raw
    // fences until the first click into the body.
    const active = view.hasFocus ? activeLines(view) : new Set<number>();
    const index = view.state.facet(docsIndexFacet);
    const cross = view.state.facet(crossRefsFacet);
    const doc = view.state.doc;
    const tree = syntaxTree(view.state);
    // Line classes accumulate here (a line can be quote + codeblock etc.).
    const lineClasses = new Map<number, string[]>();
    const addLineClass = (lineFrom: number, cls: string) => {
      const list = lineClasses.get(lineFrom) ?? [];
      if (!list.includes(cls)) list.push(cls);
      lineClasses.set(lineFrom, list);
    };
    const eachLine = (from: number, to: number, f: (lineFrom: number) => void) => {
      let n = doc.lineAt(from).number;
      const last = doc.lineAt(to).number;
      for (; n <= last; n++) f(doc.line(n).from);
    };
    // A construct is revealed (syntax marks visible, widgets off) when any of
    // its lines is touched by the selection.
    const revealed = (from: number, to: number) => {
      const a = doc.lineAt(from).number;
      const b = doc.lineAt(to).number;
      for (let n = a; n <= b; n++) if (active.has(n)) return true;
      return false;
    };
    // Hide a syntax mark, or dim it when its construct is revealed. Marks that
    // span a line break only get dimmed — a ViewPlugin replace decoration
    // across a newline makes CM throw.
    const markOrHide = (from: number, to: number, isRevealed: boolean) => {
      if (from >= to) return;
      const multiline = doc.lineAt(from).number !== doc.lineAt(to).number;
      decos.push(isRevealed || multiline ? markDim.range(from, to) : hide.range(from, to));
    };

    // Frontmatter: markdown has no node for it (the fences parse as a
    // horizontal rule + setext heading, which reads wrong). Display belongs
    // to the properties card (fmEditor.ts, a block widget); this pass only
    // computes the range so the generic handlers and the tag scan stay out
    // of it entirely.
    let fmLastLine = 0; // 1-based line of the closing fence; 0 = none
    if (doc.lines >= 2 && doc.line(1).text.trim() === "---") {
      const head: string[] = [];
      const max = Math.min(doc.lines, 100);
      for (let n = 1; n <= max; n++) head.push(doc.line(n).text);
      const fm = parseFrontmatter(head);
      if (fm) fmLastLine = fm.end;
    }
    const fmEndPos = fmLastLine ? doc.line(fmLastLine).to : 0;

    for (const { from, to } of view.visibleRanges) {
      tree.iterate({
        from, to,
        enter: (node) => {
          const name = node.name;

          // Nodes inside the frontmatter range are styled (or revealed raw)
          // above — never as rules/headings/paragraphs.
          if (fmEndPos && node.from < fmEndPos && node.to <= fmEndPos && name !== "Document") {
            return false;
          }

          const headingLevel = name.startsWith("ATXHeading") ? Number(name.slice(10))
            : name === "SetextHeading1" ? 1 : name === "SetextHeading2" ? 2 : 0;
          if (headingLevel) {
            eachLine(node.from, node.to, (lf) => addLineClass(lf, `lp-h lp-h${headingLevel}`));
            const rev = revealed(node.from, node.to);
            const mark = node.node.getChild("HeaderMark");
            if (mark) {
              if (name.startsWith("ATXHeading")) {
                // Swallow the trailing space after "##" too.
                const after = doc.sliceString(mark.to, mark.to + 1) === " " ? mark.to + 1 : mark.to;
                markOrHide(mark.from, after, rev);
              } else {
                // Setext underline: always visible, dimmed.
                decos.push(markDim.range(mark.from, mark.to));
              }
            }
            return;
          }

          switch (name) {
            case "Emphasis":
            case "StrongEmphasis":
            case "Strikethrough": {
              const cls = name === "Emphasis" ? "lp-em" : name === "StrongEmphasis" ? "lp-strong" : "lp-strike";
              decos.push(Decoration.mark({ class: cls }).range(node.from, node.to));
              const rev = revealed(node.from, node.to);
              const markName = name === "Strikethrough" ? "StrikethroughMark" : "EmphasisMark";
              for (const m of node.node.getChildren(markName)) markOrHide(m.from, m.to, rev);
              return;
            }
            case "Highlight": {
              decos.push(Decoration.mark({ class: "lp-highlight" }).range(node.from, node.to));
              const rev = revealed(node.from, node.to);
              for (const m of node.node.getChildren("HighlightMark")) markOrHide(m.from, m.to, rev);
              return false; // marks handled; the inner text parses as plain
            }
            case "InlineCode": {
              decos.push(Decoration.mark({ class: "lp-code" }).range(node.from, node.to));
              const rev = revealed(node.from, node.to);
              for (const m of node.node.getChildren("CodeMark")) markOrHide(m.from, m.to, rev);
              return false; // don't descend: backticks handled, content is literal
            }
            case "Link": {
              decos.push(Decoration.mark({ class: "lp-link", attributes: { title: "⌘-click to open" } }).range(node.from, node.to));
              const rev = revealed(node.from, node.to);
              for (const m of node.node.getChildren("LinkMark")) {
                // Keep the "[" "]" pair hidden along with "(url)".
                markOrHide(m.from, m.to, rev);
              }
              for (const part of ["URL", "LinkTitle"]) {
                for (const m of node.node.getChildren(part)) markOrHide(m.from, m.to, rev);
              }
              return;
            }
            case "Wikilink": {
              const rev = revealed(node.from, node.to);
              const text = doc.sliceString(node.from + 2, node.to - 2);
              const pipe = text.indexOf("|");
              const targetFull = pipe >= 0 ? text.slice(0, pipe) : text;
              const hashAt = targetFull.indexOf("#");
              const target = (hashAt >= 0 ? targetFull.slice(0, hashAt) : targetFull).trim();
              const view = wikilinkView(index, cross, target);
              decos.push(Decoration.mark({
                class: `lp-wikilink${view.unresolved ? " unresolved" : ""}`,
                attributes: { title: view.title },
              }).range(node.from, node.to));
              const marks = node.node.getChildren("WikilinkMark");
              if (marks.length === 2) {
                markOrHide(marks[0].from, marks[0].to, rev);
                // With an alias, hide "target|" so only the alias shows.
                if (pipe >= 0 && !rev) {
                  markOrHide(node.from + 2, node.from + 2 + pipe + 1, false);
                } else if (pipe >= 0) {
                  decos.push(markDim.range(node.from + 2, node.from + 2 + pipe + 1));
                }
                markOrHide(marks[1].from, marks[1].to, rev);
              }
              return false;
            }
            case "ListMark": {
              const parent = node.node.parent;
              if (parent?.name !== "ListItem" || revealed(node.from, node.to)) return;
              const text = doc.sliceString(node.from, node.to);
              if (!"-*+".includes(text)) return; // ordered-list numbers stay
              const isTask = parent.getChild("Task") !== null;
              const deco = isTask
                ? hide // task items render as bare checkboxes, no bullet
                : Decoration.replace({ widget: new BulletWidget() });
              decos.push(deco.range(node.from, node.to));
              if (!isTask) atomics.push(deco.range(node.from, node.to));
              return;
            }
            case "TaskMarker": {
              if (revealed(node.from, node.to)) return;
              const checked = /x/i.test(doc.sliceString(node.from, node.to));
              const deco = Decoration.replace({ widget: new CheckboxWidget(checked) });
              decos.push(deco.range(node.from, node.to));
              atomics.push(deco.range(node.from, node.to));
              return;
            }
            case "Image": {
              const urlNode = node.node.getChild("URL");
              if (!urlNode) return;
              // A replace decoration from a ViewPlugin may not span a line
              // break (CM throws) — multi-line alt text stays as raw syntax.
              if (doc.lineAt(node.from).number !== doc.lineAt(node.to).number) return;
              const src = doc.sliceString(urlNode.from, urlNode.to);
              // Remote images can't load under the CSP; only embed local ones.
              if (/^[a-z][a-z0-9+.-]*:/i.test(src)) return;
              if (revealed(node.from, node.to)) return;
              const nav = view.state.facet(docsNavFacet);
              if (!nav) return;
              const altEnd = doc.sliceString(node.from, node.to).indexOf("]");
              const alt = altEnd > 2 ? doc.sliceString(node.from + 2, node.from + altEnd) : "";
              const deco = Decoration.replace({ widget: new ImageWidget(src, alt, nav) });
              decos.push(deco.range(node.from, node.to));
              atomics.push(deco.range(node.from, node.to));
              return false;
            }
            case "Blockquote": {
              const firstLine = doc.lineAt(node.from);
              const head = doc.sliceString(node.from, firstLine.to);
              const callout = /^>\s*\[!(\w+)\]/.exec(head);
              if (callout) {
                const kind = CALLOUT_TYPES[callout[1].toLowerCase()] ?? "note";
                eachLine(node.from, node.to, (lf) => addLineClass(lf, `lp-quote lp-callout lp-callout-${kind}`));
                const start = node.from + head.indexOf("[!");
                const end = start + callout[1].length + 3; // "[!" + type + "]"
                if (revealed(node.from, firstLine.to)) {
                  decos.push(markDim.range(start, end));
                } else {
                  const label = callout[1][0].toUpperCase() + callout[1].slice(1).toLowerCase();
                  const deco = Decoration.replace({ widget: new CalloutTitleWidget(kind, label) });
                  decos.push(deco.range(start, end));
                  atomics.push(deco.range(start, end));
                }
              } else {
                eachLine(node.from, node.to, (lf) => addLineClass(lf, "lp-quote"));
              }
              return;
            }
            case "QuoteMark": {
              markOrHide(node.from, node.to, revealed(node.from, node.to));
              return;
            }
            case "CodeBlock": {
              // Indented (4-space) code: no fences to hide, but it earns the
              // same box as a fenced block instead of reading as prose.
              const first = doc.lineAt(node.from);
              const last = doc.lineAt(node.to);
              for (let n = first.number; n <= last.number; n++) {
                addLineClass(doc.line(n).from, "lp-codeblock");
              }
              addLineClass(first.from, "lp-cb-open");
              addLineClass(last.from, "lp-cb-close");
              return false; // its content is literal
            }
            case "FencedCode": {
              // The fence lines carry the block's chrome rather than its
              // syntax: the opening one becomes a header strip (language +
              // copy button), the closing one an empty strip that reads as
              // padding. Reveal is per fence line, not per block, so the
              // header survives while you edit the code under it.
              const first = doc.lineAt(node.from);
              const last = doc.lineAt(node.to);
              // An unterminated block (still being typed, or running to EOF)
              // has no closing fence — its last line is code.
              const closed = last.number > first.number && FENCE_CLOSE_RE.test(last.text);
              const lastBody = closed ? last.number - 1 : last.number;
              let anyCode = false;
              for (let n = first.number; n <= last.number; n++) {
                const line = doc.line(n);
                addLineClass(line.from, "lp-codeblock");
                if (n > first.number && n <= lastBody && line.text.trim() !== "") anyCode = true;
              }
              addLineClass(first.from, "lp-cb-open");
              addLineClass(last.from, "lp-cb-close");

              // The regex misses a fence nested in a blockquote (the line
              // starts with its quote marks, which carry decorations of their
              // own); those keep the raw fences, merely dimmed.
              const open = FENCE_OPEN_RE.exec(first.text);
              if (open && !revealed(first.from, first.to)) {
                const lang = open[3].trim().split(/\s+/)[0] ?? "";
                const deco = Decoration.replace({ widget: new CodeHeaderWidget(lang, anyCode) });
                decos.push(deco.range(first.from, first.to));
                atomics.push(deco.range(first.from, first.to));
              } else {
                addLineClass(first.from, "lp-fence");
              }

              if (closed) {
                if (revealed(last.from, last.to)) addLineClass(last.from, "lp-fence");
                else if (last.to > last.from) decos.push(hide.range(last.from, last.to));
              }
              return; // descend: nested language highlighting still applies
            }
            case "Table": {
              // Rendering belongs to the block widget in mdTable.ts. When the
              // caret is inside, that widget steps aside and these dress the
              // raw rows it leaves behind: monospace so the pipes line up as
              // columns while they are being edited.
              if (revealed(node.from, node.to)) {
                eachLine(node.from, node.to, (lf) => addLineClass(lf, "lp-table-raw"));
              }
              return; // descend: cells hold ordinary inline markdown
            }
            case "TableDelimiter": {
              // Only while revealed. Otherwise the widget covers them, and a
              // table nested in a blockquote (which gets no widget) reads
              // better with its pipes at full strength.
              if (revealed(node.from, node.to)) decos.push(markDim.range(node.from, node.to));
              return;
            }
            case "HorizontalRule": {
              if (revealed(node.from, node.to)) return;
              const deco = Decoration.replace({ widget: new HRWidget() });
              decos.push(deco.range(node.from, node.to));
              atomics.push(deco.range(node.from, node.to));
              return;
            }
          }
          return;
        },
      });

      // #tags aren't parse nodes — regex over the visible lines, skipping code.
      let n = doc.lineAt(from).number;
      const last = doc.lineAt(to).number;
      for (; n <= last; n++) {
        if (n <= fmLastLine) continue; // frontmatter carries no tags
        const line = doc.line(n);
        for (const m of line.text.matchAll(TAG_RE)) {
          const start = line.from + m.index + m[1].length;
          const inner = tree.resolveInner(start + 1, 1);
          if (/Code|Comment/.test(inner.name) || inner.name === "FencedCode") continue;
          decos.push(Decoration.mark({ class: "lp-tag" }).range(start, start + m[2].length + 1));
        }
      }
    }

    for (const [lineFrom, classes] of lineClasses) {
      decos.push(Decoration.line({ class: classes.join(" ") }).range(lineFrom));
    }

    return [
      Decoration.set(decos, true),
      RangeSet.of(atomics, true),
    ];
  }
}

// ── Click handling: ⌘-click wikilinks & links, plain-click tags ──────────────

function handleMouse(e: MouseEvent, view: EditorView): boolean {
  const nav = view.state.facet(docsNavFacet);
  if (!nav) return false;

  const target = e.target as HTMLElement | null;
  const tagEl = target?.closest?.(".lp-tag");
  if (tagEl && !(e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    nav.onTagClick(tagEl.textContent ?? "");
    return true;
  }

  if (!(e.metaKey || e.ctrlKey)) return false;
  const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
  if (pos === null) return false;
  const inner = syntaxTree(view.state).resolveInner(pos, 0);
  for (let node: typeof inner | null = inner; node; node = node.parent) {
    if (node.name === "Wikilink") {
      e.preventDefault();
      const text = view.state.doc.sliceString(node.from + 2, node.to - 2);
      const pipe = text.indexOf("|");
      const targetFull = (pipe >= 0 ? text.slice(0, pipe) : text).trim();
      const hashAt = targetFull.indexOf("#");
      nav.onNavigate(
        hashAt >= 0 ? targetFull.slice(0, hashAt).trim() : targetFull,
        hashAt >= 0 ? targetFull.slice(hashAt + 1).trim() || null : null,
      );
      return true;
    }
    if (node.name === "Link" || node.name === "Autolink" || node.name === "URL") {
      const urlNode = node.name === "URL" ? node : node.getChild("URL");
      if (!urlNode) continue;
      const href = view.state.doc.sliceString(urlNode.from, urlNode.to);
      if (/^https?:\/\//.test(href)) {
        e.preventDefault();
        void openUrl(href).catch(() => {});
        return true;
      }
      return false;
    }
  }
  return false;
}

// ── Autocomplete: [[note targets and #tags from the index ────────────────────

function wikilinkCompletions(ctx: CompletionContext): CompletionResult | null {
  const index = ctx.state.facet(docsIndexFacet);
  if (!index) return null;
  const m = ctx.matchBefore(/\[\[[^\]\n]*$/);
  if (!m) return null;
  const options = [...index.docs.values()].map((d) => {
    // Ambiguous basenames complete as their path form so the link stays exact.
    const dup = (index.byBase.get(d.base.toLowerCase()) ?? []).length > 1;
    const label = dup ? stripExt(d.path) : d.base;
    return { label, detail: d.title !== d.base ? d.title : d.path, apply: `${label}]]` };
  });
  // Issues complete too ([[AGE-14 → the tracker); notes win ties via boost.
  const cross = ctx.state.facet(crossRefsFacet);
  if (cross) {
    options.push(...issueCompletionOptions(cross).map((o) => ({
      label: o.label, detail: o.detail, apply: `${o.label}]]`, boost: -1,
    })));
  }
  return { from: m.from + 2, options, validFor: /^[^\]\n]*$/ };
}

function tagCompletions(ctx: CompletionContext): CompletionResult | null {
  const index = ctx.state.facet(docsIndexFacet);
  if (!index || index.tags.size === 0) return null;
  const m = ctx.matchBefore(/#[\w/-]*$/);
  if (!m) return null;
  // Only at a word boundary — "foo#bar" is not a tag.
  if (m.from > 0 && /[^\s(]/.test(ctx.state.doc.sliceString(m.from - 1, m.from))) return null;
  return {
    from: m.from,
    options: [...index.tags.keys()].map((t) => ({ label: `#${t}` })),
    validFor: /^#[\w/-]*$/,
  };
}

/** Completion for the docs editor: wikilink + tag sources only (no word noise). */
export const docsCompletion: Extension = autocompletion({
  override: [wikilinkCompletions, tagCompletions],
  icons: false,
});

const livePreviewPlugin = ViewPlugin.fromClass(LivePreviewPlugin, {
  decorations: (v) => v.decorations,
  provide: (p) => EditorView.atomicRanges.of((view) => view.plugin(p)?.atomic ?? RangeSet.empty),
  eventHandlers: { mousedown: handleMouse },
});

/** The full live-preview extension bundle (decorations + clicks). */
export const livePreview: Extension = [livePreviewPlugin];
