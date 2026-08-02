import { Extension, Facet, Range, RangeSet } from "@codemirror/state";
import {
  Decoration, DecorationSet, EditorView, ViewPlugin, ViewUpdate, WidgetType,
} from "@codemirror/view";
import { HighlightStyle, LanguageDescription, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import type { MarkdownConfig, InlineContext } from "@lezer/markdown";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { rust } from "@codemirror/lang-rust";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { python } from "@codemirror/lang-python";
import { autocompletion, CompletionContext, CompletionResult } from "@codemirror/autocomplete";
import { openUrl } from "@tauri-apps/plugin-opener";
import { FileRoot, readFileBase64 } from "../api";
import { DocsIndex, parseFrontmatter, stripExt } from "./docsIndex";
import { CrossRefs, issueCompletionOptions, wikilinkView } from "./links";
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

const codeLanguages = [
  LanguageDescription.of({
    name: "javascript",
    alias: ["js", "jsx", "ts", "tsx", "typescript"],
    load: async () => javascript({ typescript: true, jsx: true }),
  }),
  LanguageDescription.of({ name: "json", load: async () => json() }),
  LanguageDescription.of({ name: "rust", alias: ["rs"], load: async () => rust() }),
  LanguageDescription.of({ name: "css", load: async () => css() }),
  LanguageDescription.of({ name: "html", load: async () => html() }),
  LanguageDescription.of({ name: "python", alias: ["py"], load: async () => python() }),
];

/** Markdown language for docs notes: GFM base + wikilinks + fenced-code langs. */
export const docsMarkdown = () =>
  markdown({ base: markdownLanguage, codeLanguages, extensions: [wikilinkExtension, highlightExtension] });

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
      if (!/^\[[ xX]\]$/.test(cur)) return;
      const insert = this.checked ? "[ ]" : "[x]";
      view.dispatch({ changes: { from: pos, to: pos + 3, insert } });
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
  toDOM() {
    const el = document.createElement("span");
    el.className = `lp-callout-title lp-callout-title-${this.kind}`;
    el.textContent = `◆ ${this.label}`;
    return el;
  }
}

class BulletWidget extends WidgetType {
  eq() { return true; }
  toDOM() {
    const el = document.createElement("span");
    el.className = "lp-bullet";
    el.textContent = "•";
    return el;
  }
}

class HRWidget extends WidgetType {
  eq() { return true; }
  toDOM() {
    const el = document.createElement("span");
    el.className = "lp-hr";
    return el;
  }
}


// ── Decoration building ──────────────────────────────────────────────────────

const TAG_RE = /(^|[\s(])#([A-Za-z0-9_][A-Za-z0-9_/-]*)/g;

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
            case "FencedCode": {
              const firstLine = doc.lineAt(node.from).number;
              const lastLine = doc.lineAt(node.to).number;
              for (let n = firstLine; n <= lastLine; n++) {
                const lf = doc.line(n).from;
                addLineClass(lf, "lp-codeblock");
                if (n === firstLine || n === lastLine) addLineClass(lf, "lp-fence");
              }
              return; // descend: nested language highlighting still applies
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
