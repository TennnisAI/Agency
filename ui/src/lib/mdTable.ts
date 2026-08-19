// GFM tables in the live preview. A block widget draws a real <table> over the
// table's line range; the caret landing anywhere in it takes the widget away
// again, so the raw pipes are what you edit. Built as a StateField rather than
// a ViewPlugin because a replace decoration that spans a line break may only
// come from the state (CodeMirror throws when a plugin supplies one) — the
// same reason the frontmatter card in fmEditor.ts is a field.

import { EditorState, Extension, Range, StateEffect, StateField } from "@codemirror/state";
import { Decoration, DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CrossRefs, wikilinkView } from "./links";
import { DocsIndex } from "./docsIndex";
import { crossRefsFacet, docsIndexFacet, DocsNav, docsNavFacet, TAG_RE } from "./livePreview";

// @lezer/common is not a direct dependency (pnpm's strict layout would not
// resolve it), so the node type comes off the tree instead.
type SyntaxNode = ReturnType<typeof syntaxTree>["topNode"];

// ── Model: what the widget draws, read straight off the document ─────────────

export type ColAlign = "left" | "center" | "right" | null;

/** One cell's content range in the document, surrounding padding trimmed off. */
export interface TableCell {
  from: number;
  to: number;
}

export interface TableRow {
  from: number;
  to: number;
  cells: TableCell[];
}

export interface TableModel {
  from: number;
  to: number;
  /** Raw text of the whole table. The widget rebuilds only when this changes. */
  text: string;
  header: TableRow | null;
  rows: TableRow[];
  align: ColAlign[];
  /** Column count, from the header row: GFM drops a row's excess cells. */
  cols: number;
}

/** Column alignments read off the `|---|:-:|---:|` separator row. */
export function parseAlign(delimiter: string): ColAlign[] {
  const parts = delimiter.split("|");
  // The outer pipes are optional in GFM and make no column of their own.
  if (parts.length > 1 && parts[0].trim() === "") parts.shift();
  if (parts.length > 1 && parts[parts.length - 1].trim() === "") parts.pop();
  return parts.map((p) => {
    const s = p.trim();
    const left = s.startsWith(":");
    const right = s.endsWith(":");
    return left && right ? "center" : right ? "right" : left ? "left" : null;
  });
}

/**
 * Split a header or body row into cells.
 *
 * The cells come from the pipe positions, not from the TableCell nodes: an
 * empty cell (`| a |  | c |`) parses as no node at all, so counting nodes
 * would silently shift every column after it one to the left.
 */
function rowCells(row: SyntaxNode, state: EditorState): TableRow {
  const doc = state.doc;
  const bounds: [number, number][] = [];
  let pos = row.from;
  for (const bar of row.getChildren("TableDelimiter")) {
    bounds.push([pos, bar.from]);
    pos = bar.to;
  }
  bounds.push([pos, row.to]);
  const blank = ([from, to]: [number, number]) => doc.sliceString(from, to).trim() === "";
  if (bounds.length > 1 && blank(bounds[0])) bounds.shift();
  if (bounds.length > 1 && blank(bounds[bounds.length - 1])) bounds.pop();
  const cells = bounds.map(([from, to]) => {
    const text = doc.sliceString(from, to);
    const lead = text.length - text.trimStart().length;
    const trail = text.length - text.trimEnd().length;
    return { from: from + lead, to: Math.max(from + lead, to - trail) };
  });
  return { from: row.from, to: row.to, cells };
}

/** The table `node` describes, or null when it cannot be drawn as one. */
export function tableModel(state: EditorState, node: SyntaxNode): TableModel | null {
  const doc = state.doc;
  // A block widget has to replace whole lines. A table nested in a blockquote
  // or a list item starts after that construct's own marks, so its node begins
  // mid-line; those keep their raw pipes rather than corrupting the layout.
  if (doc.lineAt(node.from).from !== node.from) return null;
  if (doc.lineAt(node.to).to !== node.to) return null;
  const headerNode = node.getChild("TableHeader");
  const header = headerNode ? rowCells(headerNode, state) : null;
  // The separator row is the only TableDelimiter that is a direct child of the
  // table; the pipes between cells hang off the header and body rows.
  const delimiter = node.getChildren("TableDelimiter")[0];
  const align = delimiter ? parseAlign(doc.sliceString(delimiter.from, delimiter.to)) : [];
  const cols = header ? header.cells.length : align.length;
  if (cols === 0) return null;
  return {
    from: node.from,
    to: node.to,
    text: doc.sliceString(node.from, node.to),
    header,
    rows: node.getChildren("TableRow").map((r) => rowCells(r, state)),
    align,
    cols,
  };
}

/** The table containing `pos`, or null when `pos` is outside one. */
export function tableModelAt(state: EditorState, pos: number): TableModel | null {
  let node: SyntaxNode | null = syntaxTree(state).resolveInner(pos, 1);
  while (node && node.name !== "Table") node = node.parent;
  return node ? tableModel(state, node) : null;
}

// ── Cell rendering: the same inline subset live preview draws elsewhere ──────

/** Syntax marks contribute nothing to a rendered cell. */
const MARKS = new Set([
  "EmphasisMark", "StrikethroughMark", "HighlightMark", "CodeMark",
  "LinkMark", "LinkTitle", "WikilinkMark",
]);

/** Node name to the element and class that carry it. */
const INLINE: Record<string, [string, string]> = {
  Emphasis: ["em", "lp-em"],
  StrongEmphasis: ["strong", "lp-strong"],
  Strikethrough: ["span", "lp-strike"],
  Highlight: ["span", "lp-highlight"],
  InlineCode: ["code", "lp-code"],
};

interface CellCtx {
  state: EditorState;
  index: DocsIndex | null;
  cross: CrossRefs | null;
}

/** Plain text, with #tags lifted out into the same pills prose gets. */
function appendText(out: HTMLElement, text: string) {
  let pos = 0;
  for (const m of text.matchAll(TAG_RE)) {
    const start = (m.index ?? 0) + m[1].length;
    if (start > pos) out.appendChild(document.createTextNode(text.slice(pos, start)));
    const tag = document.createElement("span");
    tag.className = "lp-tag";
    tag.textContent = `#${m[2]}`;
    out.appendChild(tag);
    pos = start + m[2].length + 1;
  }
  if (pos < text.length) out.appendChild(document.createTextNode(text.slice(pos)));
}

function linkSpan(text: string, href: string): HTMLElement {
  const el = document.createElement("span");
  el.className = "lp-link";
  el.title = "⌘-click to open";
  el.dataset.href = href;
  el.textContent = text;
  return el;
}

/** Render the document range [from, to) of `parent`'s content into `out`. */
function renderRange(parent: SyntaxNode, from: number, to: number, ctx: CellCtx, out: HTMLElement) {
  const doc = ctx.state.doc;
  let pos = from;
  for (let child = parent.firstChild; child; child = child.nextSibling) {
    if (child.to <= from) continue;
    if (child.from >= to) break;
    if (child.from > pos) appendText(out, doc.sliceString(pos, child.from));
    renderNode(child, ctx, out);
    pos = child.to;
  }
  if (pos < to) appendText(out, doc.sliceString(pos, to));
}

function renderNode(node: SyntaxNode, ctx: CellCtx, out: HTMLElement) {
  const doc = ctx.state.doc;
  const name = node.name;
  if (MARKS.has(name)) return;

  if (name === "URL") {
    // A link's destination is not shown; a bare URL is the link itself (GFM
    // autolinking parses one straight into the cell).
    const parent = node.parent?.name;
    if (parent === "Link" || parent === "Image") return;
    const url = doc.sliceString(node.from, node.to);
    out.appendChild(linkSpan(url, url));
    return;
  }
  if (name === "Escape") {
    // "\|" is how a literal pipe survives a cell; only the escaped char shows.
    out.appendChild(document.createTextNode(doc.sliceString(node.from + 1, node.to)));
    return;
  }
  if (name === "Wikilink") {
    const raw = doc.sliceString(node.from + 2, node.to - 2);
    const pipe = raw.indexOf("|");
    const targetFull = pipe >= 0 ? raw.slice(0, pipe) : raw;
    const hashAt = targetFull.indexOf("#");
    const target = (hashAt >= 0 ? targetFull.slice(0, hashAt) : targetFull).trim();
    const resolved = wikilinkView(ctx.index, ctx.cross, target);
    const el = document.createElement("span");
    el.className = `lp-wikilink${resolved.unresolved ? " unresolved" : ""}`;
    el.title = resolved.title;
    el.dataset.wikilink = targetFull;
    el.textContent = pipe >= 0 ? raw.slice(pipe + 1) : raw;
    out.appendChild(el);
    return;
  }
  if (name === "Link") {
    const urlNode = node.getChild("URL");
    const el = document.createElement("span");
    el.className = "lp-link";
    el.title = "⌘-click to open";
    if (urlNode) el.dataset.href = doc.sliceString(urlNode.from, urlNode.to);
    renderRange(node, node.from, node.to, ctx, el);
    out.appendChild(el);
    return;
  }
  const inline = INLINE[name];
  if (inline) {
    const el = document.createElement(inline[0]);
    el.className = inline[1];
    renderRange(node, node.from, node.to, ctx, el);
    out.appendChild(el);
    return;
  }
  // Anything else — an Autolink's brackets, an Image, a construct with no
  // rendering of its own — is transparent: marks drop out, text comes through.
  renderRange(node, node.from, node.to, ctx, out);
}

/** Draw a cell's content, or leave it empty when there is nothing to draw. */
function renderCell(ctx: CellCtx, cell: TableCell, out: HTMLElement) {
  if (cell.from >= cell.to) return;
  let node: SyntaxNode | null = syntaxTree(ctx.state).resolveInner(cell.from, 1);
  while (node && node.name !== "TableCell") node = node.parent;
  if (node) renderRange(node, node.from, node.to, ctx, out);
  else appendText(out, ctx.state.doc.sliceString(cell.from, cell.to));
}

// ── The widget ───────────────────────────────────────────────────────────────

/** Follow a rendered link or wikilink; false when there is nowhere to go. */
function follow(el: HTMLElement, nav: DocsNav | null): boolean {
  const wl = el.dataset.wikilink;
  if (wl !== undefined) {
    if (!nav) return false;
    const hashAt = wl.indexOf("#");
    nav.onNavigate(
      hashAt >= 0 ? wl.slice(0, hashAt).trim() : wl.trim(),
      hashAt >= 0 ? wl.slice(hashAt + 1).trim() || null : null,
    );
    return true;
  }
  const href = el.dataset.href;
  if (href && /^https?:\/\//.test(href)) {
    void openUrl(href).catch(() => {});
    return true;
  }
  return false;
}

class TableWidget extends WidgetType {
  constructor(
    readonly model: TableModel,
    readonly index: DocsIndex | null,
    readonly cross: CrossRefs | null,
  ) { super(); }

  eq(other: TableWidget) {
    // The index and the cross-refs matter too: they decide whether a wikilink
    // in a cell draws as resolved.
    return other.model.text === this.model.text
      && other.index === this.index && other.cross === this.cross;
  }

  ignoreEvent() { return true; } // the table owns its clicks

  get estimatedHeight() { return 30 * (this.model.rows.length + 1) + 10; }

  toDOM(view: EditorView) {
    const model = this.model;
    const ctx: CellCtx = { state: view.state, index: this.index, cross: this.cross };
    const wrap = document.createElement("div");
    wrap.className = "lp-table-wrap";
    wrap.setAttribute("contenteditable", "false");
    const table = document.createElement("table");
    table.className = "lp-table";
    wrap.appendChild(table);

    const drawRow = (row: TableRow, tag: "th" | "td"): HTMLTableRowElement => {
      const tr = document.createElement("tr");
      for (let c = 0; c < model.cols; c++) {
        const el = document.createElement(tag);
        const align = model.align[c];
        if (align) el.style.textAlign = align;
        const cell = row.cells[c];
        // The caret target is an offset from the table's start, not a document
        // position: an edit ABOVE the table leaves the table's own text alone,
        // so eq() holds and CodeMirror reuses this DOM with every position in
        // it shifted. posAtDOM re-bases the offset at click time.
        // A short row's missing cells still take a caret: clicking one parks
        // it at the end of that row, which is where the text would go.
        el.dataset.at = String((cell ? cell.to : row.to) - model.from);
        if (cell) renderCell(ctx, cell, el);
        tr.appendChild(el);
      }
      return tr;
    };

    if (model.header) {
      const thead = document.createElement("thead");
      thead.appendChild(drawRow(model.header, "th"));
      table.appendChild(thead);
    }
    const tbody = document.createElement("tbody");
    for (const row of model.rows) tbody.appendChild(drawRow(row, "td"));
    table.appendChild(tbody);

    wrap.addEventListener("mousedown", (e) => {
      const target = e.target as HTMLElement | null;
      const nav = view.state.facet(docsNavFacet);
      const tag = nav && target?.closest?.(".lp-tag");
      if (tag && !(e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        nav.onTagClick(tag.textContent ?? "");
        return;
      }
      if (e.metaKey || e.ctrlKey) {
        const link = target?.closest?.(".lp-link, .lp-wikilink") as HTMLElement | null;
        if (link && follow(link, nav)) {
          e.preventDefault();
          return;
        }
      }
      // Anything else is a request to edit: park the caret in the cell that
      // was clicked, which is what puts the raw rows back.
      e.preventDefault();
      const cell = target?.closest?.("[data-at]") as HTMLElement | null;
      const at = view.posAtDOM(wrap)
        + (cell ? Number(cell.dataset.at) : model.to - model.from);
      view.dispatch({ selection: { anchor: Math.min(at, view.state.doc.length) } });
      view.focus();
    });

    return wrap;
  }
}

// ── The field ────────────────────────────────────────────────────────────────

/** True while any selection range reaches into the table's line range. */
function selectionTouches(state: EditorState, from: number, to: number): boolean {
  for (const r of state.selection.ranges) if (r.from <= to && r.to >= from) return true;
  return false;
}

function buildTables(state: EditorState, focused: boolean): DecorationSet {
  const index = state.facet(docsIndexFacet);
  const cross = state.facet(crossRefsFacet);
  const decos: Range<Decoration>[] = [];
  // Only top-level tables get a widget (tableModel rejects the nested ones
  // anyway), so the walk stays over the document's own children rather than
  // descending the whole tree on every keystroke.
  for (let node = syntaxTree(state).topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== "Table") continue;
    const model = tableModel(state, node);
    if (!model) continue;
    if (focused && selectionTouches(state, model.from, model.to)) continue;
    decos.push(Decoration.replace({
      widget: new TableWidget(model, index, cross), block: true,
    }).range(model.from, model.to));
  }
  return Decoration.set(decos, true);
}

const setFocus = StateEffect.define<boolean>();

interface TableState {
  focused: boolean;
  decos: DecorationSet;
}

// Reveal is focus-gated for the reason it is in livePreview.ts: an unfocused
// editor shows no caret, so nothing should be showing raw syntax. Without the
// gate a note whose first line is a table opens as raw pipes, because the
// caret parks at position 0 on mount. A StateField cannot read view.hasFocus,
// so focusChangeEffect feeds the focus flag into the state instead.
const tableField = StateField.define<TableState>({
  create: (state) => ({ focused: false, decos: buildTables(state, false) }),
  update(value, tr) {
    let focused = value.focused;
    for (const e of tr.effects) if (e.is(setFocus)) focused = e.value;
    // The parse runs in the background on a long document, so a table below
    // the first chunk only becomes a node some transactions later.
    const treeGrew = syntaxTree(tr.startState) !== syntaxTree(tr.state);
    const refsChanged =
      tr.startState.facet(docsIndexFacet) !== tr.state.facet(docsIndexFacet) ||
      tr.startState.facet(crossRefsFacet) !== tr.state.facet(crossRefsFacet);
    if (!tr.docChanged && !tr.selection && !treeGrew && !refsChanged && focused === value.focused) {
      return value;
    }
    return { focused, decos: buildTables(tr.state, focused) };
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
});

/**
 * Rendered GFM tables for the docs note editor and the issue description.
 *
 * Deliberately not atomic, unlike the frontmatter card: arrowing down into the
 * widget has to land the caret inside the table, because that is what puts the
 * raw rows back. Atomic ranges would hop the whole table and leave no keyboard
 * way in.
 */
export const markdownTables: Extension = [
  tableField,
  EditorView.focusChangeEffect.of((_state, focusing) => setFocus.of(focusing)),
];
