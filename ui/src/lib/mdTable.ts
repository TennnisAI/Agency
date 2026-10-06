// GFM tables in the live preview. A block widget draws a real <table> over the
// table's line range, and it stays drawn while you work in it: a cell is edited
// in place, by a small editor of its own mounted in that one cell.
//
// The table used to step aside for its raw pipes the moment the caret touched
// it, and a press is what puts a caret down. So pressing in a cell to start a
// selection turned the whole table back into markdown under the pointer: the
// columns collapsed into monospace pipe rows, and the text the user meant to
// select was no longer where they had pressed (AGE-254).
//
// The widget comes from a StateField rather than a ViewPlugin because a replace
// decoration that spans a line break may only come from the state (CodeMirror
// throws when a plugin supplies one), the same reason the frontmatter card in
// fmEditor.ts is a field.

import {
  Annotation, ChangeSpec, EditorSelection, EditorState, Extension, Prec, Range, StateEffect, StateField,
  Text, Transaction, TransactionSpec,
} from "@codemirror/state";
import {
  Decoration, DecorationSet, EditorView, ViewPlugin, ViewUpdate, WidgetType, drawSelection,
  keymap, runScopeHandlers,
} from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";
import { redo, standardKeymap, undo } from "@codemirror/commands";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CrossRefs, wikilinkView } from "./links";
import { DocsIndex } from "./docsIndex";
import {
  crossRefsFacet, docsCompletion, docsHighlight, docsIndexFacet, docsMarkdown, DocsNav,
  docsNavFacet, livePreview, TAG_RE,
} from "./livePreview";
import { pasteImages } from "./imagePaste";
import { minimalReplacement } from "./textEdit";

// @lezer/common is not a direct dependency (pnpm's strict layout would not
// resolve it), so the node type comes off the tree instead.
type SyntaxNode = ReturnType<typeof syntaxTree>["topNode"];

// ── Model: what the widget draws, read straight off the document ─────────────

export type ColAlign = "left" | "center" | "right" | null;

export interface TableCell {
  /** The content's range, surrounding padding trimmed off. An empty cell's
   *  range is the point a caret should sit at to type into it. */
  from: number;
  to: number;
  /** Everything between the cell's pipes, padding included. */
  start: number;
  end: number;
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
  const cells = bounds.map(([start, end]) => {
    const text = doc.sliceString(start, end);
    if (text.trim() === "") {
      // One space in, so typing into "|  |" makes "| x |" rather than "|  x|".
      const at = Math.min(start + 1, end);
      return { from: at, to: at, start, end };
    }
    const lead = text.length - text.trimStart().length;
    const trail = text.length - text.trimEnd().length;
    return { from: start + lead, to: end - trail, start, end };
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

/** The table containing `pos`, or null when `pos` is outside one. `side`
 *  picks the node on that side of `pos` when it sits on a boundary. */
export function tableModelAt(state: EditorState, pos: number, side: -1 | 1 = 1): TableModel | null {
  let node: SyntaxNode | null = syntaxTree(state).resolveInner(pos, side);
  while (node && node.name !== "Table") node = node.parent;
  return node ? tableModel(state, node) : null;
}

/** The table `pos` is in or at either edge of. */
function tableNear(state: EditorState, pos: number): TableModel | null {
  const model = tableModelAt(state, pos, 1) ?? tableModelAt(state, pos, -1);
  return model && pos >= model.from && pos <= model.to ? model : null;
}

// ── Addressing cells ─────────────────────────────────────────────────────────

/** A cell by position: row 0 is the header, then the body rows in order. */
export interface CellRef {
  row: number;
  col: number;
}

const sameRef = (a: CellRef, b: CellRef) => a.row === b.row && a.col === b.col;

/** Header first, then the body rows: the order cells are addressed in. */
export function gridRows(model: TableModel): TableRow[] {
  return model.header ? [model.header, ...model.rows] : model.rows;
}

/** The cell `ref` names, or null when its row is too short to have one. */
export function cellOf(model: TableModel, ref: CellRef): TableCell | null {
  if (ref.col >= model.cols) return null;
  return gridRows(model)[ref.row]?.cells[ref.col] ?? null;
}

/** The cell whose span (padding included) holds all of [from, to]. */
export function cellAt(model: TableModel, from: number, to = from): CellRef | null {
  const rows = gridRows(model);
  for (let row = 0; row < rows.length; row++) {
    const r = rows[row];
    if (to < r.from || from > r.to) continue;
    const n = Math.min(r.cells.length, model.cols);
    for (let col = 0; col < n; col++) {
      const c = r.cells[col];
      if (from >= c.start && to <= c.end) return { row, col };
    }
  }
  return null;
}

/** True when `text` ends in an unpaired backslash, which escapes whatever
 *  comes after it. */
export function endsEscaped(text: string): boolean {
  let run = 0;
  for (let i = text.length - 1; i >= 0 && text[i] === "\\"; i--) run++;
  return run % 2 === 1;
}

export interface CellRepair {
  from: number;
  to: number;
  insert: string;
}

/**
 * The edits that make `text` fit in one cell: a space for each line break, and
 * a backslash in front of each pipe that would end the cell. Backslashes pair
 * up the way the table parser reads them, so the pipe in `\\|` is bare: the
 * first backslash escapes the second. `escaped` says whether the text in front
 * of `text` leaves its first character escaped.
 */
export function cellRepairs(text: string, escaped = false): CellRepair[] {
  const out: CellRepair[] = [];
  let esc = escaped;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === "\n" || ch === "\r") {
      const to = ch === "\r" && text[i + 1] === "\n" ? i + 2 : i + 1;
      out.push({ from: i, to, insert: " " });
      i = to - 1;
      esc = false;
      continue;
    }
    if (ch === "|" && !esc) out.push({ from: i, to: i, insert: "\\" });
    esc = !esc && ch === "\\";
  }
  return out;
}

/**
 * The escaped pipe ("\|") a Backspace (`dir` -1) or a Delete (1) at `pos` in
 * a cell's text would cut in half, as the range to delete whole. A caret
 * between the backslash and the pipe takes both either way.
 *
 * Deleting just the backslash left a bare pipe, which a cell cannot hold, so
 * the cell's input filter put the escape straight back: Backspace there only
 * moved the caret, and Delete did nothing at all.
 */
export function escapedPipeAt(text: string, pos: number, dir: 1 | -1): { from: number; to: number } | null {
  const pipeAt = (i: number) => text[i] === "|" && text[i - 1] === "\\" && endsEscaped(text.slice(0, i));
  if (pipeAt(pos)) return { from: pos - 1, to: pos + 1 };
  if (dir < 0 && pos >= 2 && pipeAt(pos - 1)) return { from: pos - 2, to: pos };
  if (dir > 0 && pipeAt(pos + 1)) return { from: pos, to: pos + 2 };
  return null;
}

/** Text bound for a cell: one line, and no pipe that would end the cell
 *  early. `before` is the cell's text in front of where it goes. */
export function cellSafe(text: string, before = ""): string {
  let out = "";
  let pos = 0;
  for (const r of cellRepairs(text, endsEscaped(before))) {
    out += text.slice(pos, r.from) + r.insert;
    pos = r.to;
  }
  return out + text.slice(pos);
}

/** True when the main selection sits inside one table cell. */
export function selectionInCell(state: EditorState): boolean {
  return selectedCell(state) !== null;
}

/** `text` made safe for wherever the selection is, when that is in a cell. */
export function cellSafeAtSelection(state: EditorState, text: string): string {
  const sel = state.selection.main;
  if (!selectionInCell(state)) return text;
  return cellSafe(text, state.sliceDoc(state.doc.lineAt(sel.from).from, sel.from));
}

/**
 * Where a selection that has landed in a rendered table should go. The main
 * editor cannot put a caret inside the widget, so it either becomes a cell
 * editor's selection, or, for a click beside the table, leaves the table.
 *
 * `prev` is where the caret came from (null when focus alone changed), which
 * decides which end of the table an arrow key enters by.
 */
export type TableEntry =
  | { kind: "cell"; ref: CellRef; anchor: number; head: number }
  | { kind: "leave"; exit: TableExit };

/** Where a caret leaving a table goes, and any text to add for it first. */
export interface TableExit {
  anchor: number;
  insert?: { from: number; text: string };
}

/**
 * Leaving a table downward. Not onto the line straight under the last row:
 * GFM reads a plain line there as one more row, so the paragraph the user went
 * down to start would have joined the table. The caret goes past a blank line,
 * making one when the table ends the document.
 */
export function exitBelow(doc: Text, model: TableModel): TableExit {
  if (model.to === doc.length) {
    return { anchor: doc.length + 2, insert: { from: doc.length, text: "\n\n" } };
  }
  const next = doc.lineAt(model.to + 1);
  // Another block (a heading, a list) ends the table on its own.
  if (next.text.trim() !== "") return { anchor: next.from };
  if (next.number === doc.lines) return { anchor: doc.length + 1, insert: { from: doc.length, text: "\n" } };
  return { anchor: doc.line(next.number + 1).from };
}

/** Leaving a table upward: the end of the line above, where a table may
 *  follow a paragraph directly. Null when the table opens the document. */
function exitAbove(model: TableModel): TableExit | null {
  return model.from > 0 ? { anchor: model.from - 1 } : null;
}

export function tableEntry(
  model: TableModel, doc: Text, anchor: number, head: number,
  prev: number | null, pointer: boolean,
): TableEntry | null {
  const from = Math.min(anchor, head);
  const to = Math.max(anchor, head);
  if (from < model.from || to > model.to) return null;
  if (pointer && anchor === head) {
    // A click in the margin beside the table, at its last line or its first,
    // or under a table that ends the note: the way to the end of it. Before
    // the cells, because a row with no closing pipe ends on its last cell's
    // text, and a click below the table opened that cell instead.
    if (head === model.to) return { kind: "leave", exit: exitBelow(doc, model) };
    const above = head === model.from && exitAbove(model);
    if (above) return { kind: "leave", exit: above };
  }
  const ref = cellAt(model, from, to);
  if (ref) {
    const cell = cellOf(model, ref)!;
    const clamp = (p: number) => Math.min(cell.to, Math.max(cell.from, p));
    return { kind: "cell", ref, anchor: clamp(anchor), head: clamp(head) };
  }
  // A range across the table's structure stays the main editor's.
  if (anchor !== head) return null;
  const rows = gridRows(model);
  const at = (row: number, col: number, end: boolean): TableEntry => {
    const cell = rows[row].cells[col];
    const p = end ? cell.to : cell.from;
    return { kind: "cell", ref: { row, col }, anchor: p, head: p };
  };
  const last = rows.length - 1;
  if (prev !== null && prev > model.to) {
    return at(last, Math.min(rows[last].cells.length, model.cols) - 1, true);
  }
  if ((prev !== null && prev < model.from) || head === model.from) return at(0, 0, false);
  // On a pipe, or on the separator row: the nearest cell on that line, the
  // header's for the separator.
  const row = Math.max(0, rows.findIndex((r) => head >= r.from && head <= r.to));
  const n = Math.min(rows[row].cells.length, model.cols);
  let col = rows[row].cells.slice(0, n).findIndex((c) => head <= c.end);
  if (col < 0) col = n - 1;
  return at(row, col, head > rows[row].cells[col].start);
}

// ── Structural edits: rows and columns ───────────────────────────────────────

/** The table and cell the main selection is in, or null outside any cell. */
function selectedCell(state: EditorState): { model: TableModel; ref: CellRef } | null {
  const sel = state.selection.main;
  const model = tableNear(state, sel.from);
  const ref = model && cellAt(model, sel.from, sel.to);
  return model && ref ? { model, ref } : null;
}

/** A row of empty cells. Each cell is two spaces, so its caret sits at
 *  offset 3 * col + 2 from the line's start. */
const blankRow = (cols: number) => "|" + "  |".repeat(cols);

export type TableEdit = "rowAbove" | "rowBelow" | "colBefore" | "colAfter" | "deleteRow" | "deleteCol";

/** What the table menu can offer for the cell the selection is in. */
export function tableEdits(state: EditorState): Set<TableEdit> | null {
  const at = selectedCell(state);
  if (!at) return null;
  const out = new Set<TableEdit>(["rowBelow", "colBefore", "colAfter"]);
  const header = at.model.header !== null && at.ref.row === 0;
  if (!header) out.add("rowAbove").add("deleteRow");
  if (at.model.cols > 1 && at.model.header) out.add("deleteCol");
  if (!at.model.header) {
    out.delete("colBefore");
    out.delete("colAfter");
  }
  return out;
}

/** Rows a table's text rewritten from a grid of cell texts, padded so the
 *  pipes line up. Row 0 is the header; the separator follows it. */
export function formatTable(grid: string[][], align: ColAlign[]): { text: string; widths: number[] } {
  const cols = align.length;
  const widths = Array.from({ length: cols }, (_, c) =>
    Math.max(3, ...grid.map((r) => (r[c] ?? "").length)));
  const line = (cells: string[]) => {
    const padded = Array.from({ length: Math.max(cols, cells.length) }, (_, c) => {
      const t = cells[c] ?? "";
      return c < cols ? t.padEnd(widths[c]) : t;
    });
    return `| ${padded.join(" | ")} |`;
  };
  const rule = align.map((a, c) => {
    const w = widths[c];
    if (a === "left") return ":" + "-".repeat(w - 1);
    if (a === "center") return ":" + "-".repeat(w - 2) + ":";
    if (a === "right") return "-".repeat(w - 1) + ":";
    return "-".repeat(w);
  });
  const text = [line(grid[0] ?? []), `| ${rule.join(" | ")} |`, ...grid.slice(1).map(line)].join("\n");
  return { text, widths };
}

/**
 * Add or remove a row or column at the cell the selection is in, leaving the
 * caret in the cell the edit is about. Null when the edit does not apply.
 *
 * Rows are inserted and deleted as single lines, so the rest of the table's
 * text is left exactly as the user wrote it. A column touches every line, and
 * the whole table is rewritten with its pipes aligned.
 */
export function editTable(state: EditorState, edit: TableEdit): TransactionSpec | null {
  const at = selectedCell(state);
  if (!at || !tableEdits(state)?.has(edit)) return null;
  const { model, ref } = at;
  const doc = state.doc;
  const rows = gridRows(model);
  const done = (spec: TransactionSpec): TransactionSpec =>
    ({ ...spec, scrollIntoView: true, userEvent: "input" });

  if (edit === "rowAbove" || edit === "rowBelow") {
    let pos: number;
    let lineStart: number;
    let insert: string;
    if (edit === "rowAbove") {
      pos = rows[ref.row].from;
      insert = blankRow(model.cols) + "\n";
      lineStart = pos;
    } else {
      // Below the header means below the separator row too.
      const after = ref.row === 0 && model.header
        ? doc.line(doc.lineAt(rows[0].to).number + 1).to
        : rows[ref.row].to;
      pos = after;
      insert = "\n" + blankRow(model.cols);
      lineStart = pos + 1;
    }
    return done({ changes: { from: pos, insert }, selection: { anchor: lineStart + 3 * ref.col + 2 } });
  }

  if (edit === "deleteRow") {
    const r = rows[ref.row];
    const cut = r.to - r.from + 1;
    const next = rows[ref.row + 1];
    const target = next ?? rows[ref.row - 1];
    const cell = target.cells[Math.min(ref.col, target.cells.length - 1)];
    return done({
      changes: { from: r.from - 1, to: r.to },
      selection: { anchor: next ? cell.from - cut : cell.from },
    });
  }

  const grid = rows.map((r) => {
    const cells = r.cells.map((c) => doc.sliceString(c.from, c.to));
    while (cells.length < model.cols) cells.push("");
    return cells;
  });
  const align = Array.from({ length: model.cols }, (_, c) => model.align[c] ?? null);
  let col: number;
  if (edit === "deleteCol") {
    for (const r of grid) r.splice(ref.col, 1);
    align.splice(ref.col, 1);
    col = Math.min(ref.col, align.length - 1);
  } else {
    col = edit === "colAfter" ? ref.col + 1 : ref.col;
    for (const r of grid) r.splice(col, 0, "");
    align.splice(col, 0, null);
  }
  const { text, widths } = formatTable(grid, align);
  // The row's line in the rewrite (the separator is line 1), then the cell's
  // place in it: each column before it takes its width plus " | ".
  const lines = text.split("\n");
  const lineNo = ref.row === 0 ? 0 : ref.row + 1;
  let offset = 0;
  for (let i = 0; i < lineNo; i++) offset += lines[i].length + 1;
  for (let c = 0; c < col; c++) offset += widths[c] + 3;
  const cellText = grid[ref.row][col] ?? "";
  return done({
    changes: { from: model.from, to: model.to, insert: text },
    selection: { anchor: model.from + offset + 2 + cellText.length },
  });
}

/** Pad a short row out to the table's width, so each column has a cell to
 *  type into. Null when the row is already full. */
export function padRow(state: EditorState, model: TableModel, row: number): TransactionSpec | null {
  const r = gridRows(model)[row];
  if (!r || r.cells.length >= model.cols) return null;
  // A row ending in an escaped pipe ("| a \|") has no closing pipe yet: that
  // one is the cell's text.
  const text = state.sliceDoc(r.from, r.to).trimEnd();
  const closed = text.endsWith("|") && !endsEscaped(text.slice(0, -1));
  const insert = (closed ? "" : " |") + "  |".repeat(model.cols - r.cells.length);
  return { changes: { from: r.to, insert }, userEvent: "input" };
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

const ctxFor = (state: EditorState): CellCtx => ({
  state, index: state.facet(docsIndexFacet), cross: state.facet(crossRefsFacet),
});

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
    // In a cell the alias pipe has to be escaped, "[[note\|alias]]", or it
    // would end the cell; the backslash is not part of the target.
    const targetFull = (pipe >= 0 ? raw.slice(0, pipe) : raw).replace(/\\$/, "");
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

// ── Editing a cell in place ──────────────────────────────────────────────────

/** On a main-editor transaction the open cell editor made. */
const fromCell = Annotation.define<true>();
/** On a cell-editor transaction that mirrors the main document. */
const fromMain = Annotation.define<true>();

/** The cell being edited in each main editor, if any. One at a time. */
const sessions = new WeakMap<EditorView, CellSession>();

/**
 * Pipes and line breaks typed or pasted into a cell would split it or end the
 * table, so they go in as "\|" and a space.
 *
 * The whole cell is checked after each edit, not just the text it inserts. A
 * deletion inserts nothing, and backspacing the backslash out of "a \| b" left
 * a bare pipe that split the cell: every later column moved one to the right
 * and the last one fell off the drawn table. A backslash typed in front of
 * "\|" did the same, by pairing with the escape. The repair puts the escape
 * back, which leaves the caret in front of it.
 */
export const cellInput = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged || tr.annotation(fromMain)) return tr;
  const repairs = cellRepairs(tr.newDoc.toString());
  if (repairs.length === 0) return tr;
  return [tr, { changes: repairs, sequential: true }];
});

/** The rendered table starting at `from`, among the main editor's blocks. */
function tableWrap(main: EditorView, from: number): HTMLElement | null {
  for (const el of Array.from(main.contentDOM.children)) {
    if (el instanceof HTMLElement && el.classList.contains("lp-table-wrap") && main.posAtDOM(el) === from) {
      return el;
    }
  }
  return null;
}

const cellSelector = (ref: CellRef) => `[data-row="${ref.row}"][data-col="${ref.col}"]`;

const refOf = (el: HTMLElement): CellRef => ({ row: Number(el.dataset.row), col: Number(el.dataset.col) });

/** The last cell of the row level with `y`: a press beside the table lands
 *  at the end of that row, the way one past the end of a line does. */
function rowEndCell(wrap: HTMLElement, y: number): HTMLElement | null {
  for (const tr of Array.from(wrap.querySelectorAll("tr"))) {
    const box = tr.getBoundingClientRect();
    if (y >= box.top && y <= box.bottom) return tr.lastElementChild as HTMLElement | null;
  }
  return null;
}

/**
 * One cell, being edited. The cell editor holds the cell's raw text and is a
 * window onto the main document: every change it makes is replayed into the
 * main editor at the cell's position, so the main editor's history, autosave
 * and find all see ordinary edits; and anything that changes the main document
 * from outside (an undo, a format command) is mirrored back. The main editor's
 * selection follows the cell's too, which is what the right-click menu and the
 * formatting keys act on.
 */
class CellSession {
  readonly cell: EditorView;
  td: HTMLElement;
  /** Where the cell's content starts in the main document. */
  base: number;
  /** The cell is one a short row does not have yet. Its row is padded out
   *  when something is written in it, not when it is opened: arrowing or
   *  clicking through a ragged table used to pad every row it passed, an
   *  edit to save and an undo step for a keypress that wrote nothing. */
  missing: boolean;
  ended = false;

  constructor(
    readonly main: EditorView,
    td: HTMLElement,
    public tableFrom: number,
    public ref: CellRef,
    content: TableCell | null,
    anchor: number,
    head: number,
  ) {
    this.td = td;
    this.missing = content === null;
    this.base = content ? content.from : -1;
    const text = content ? main.state.sliceDoc(content.from, content.to) : "";
    const clamp = (p: number) => (content ? Math.min(text.length, Math.max(0, p - content.from)) : 0);
    td.replaceChildren();
    td.classList.add("editing");
    this.cell = new EditorView({
      parent: td,
      state: EditorState.create({
        doc: text,
        selection: EditorSelection.single(clamp(anchor), clamp(head)),
        extensions: this.extensions(),
      }),
      dispatchTransactions: (trs, view) => {
        view.update(trs);
        if (!this.ended) this.toMain(trs);
      },
    });
  }

  private extensions(): Extension[] {
    const main = this.main;
    return [
      drawSelection(),
      EditorView.lineWrapping,
      docsHighlight,
      docsMarkdown(),
      livePreview,
      docsCompletion,
      docsIndexFacet.of(main.state.facet(docsIndexFacet)),
      crossRefsFacet.of(main.state.facet(crossRefsFacet)),
      docsNavFacet.of(main.state.facet(docsNavFacet)),
      cellInput,
      EditorView.contentAttributes.of({ "aria-label": "Table cell" }),
      Prec.high(keymap.of([
        { key: "Tab", run: () => this.tab(1), shift: () => this.tab(-1) },
        { key: "Enter", run: () => this.enter(1), shift: () => this.enter(-1) },
        { key: "ArrowUp", run: () => this.vertical(-1) },
        { key: "ArrowDown", run: () => this.vertical(1) },
        { key: "ArrowLeft", run: () => this.horizontal(-1) },
        { key: "ArrowRight", run: () => this.horizontal(1) },
        { key: "Backspace", run: () => this.deletePipe(-1) },
        { key: "Delete", run: () => this.deletePipe(1) },
        // The history is the main editor's: the cell's edits are its edits.
        { key: "Mod-z", run: () => undo(main), preventDefault: true },
        { key: "Mod-Shift-z", run: () => redo(main), preventDefault: true },
        { key: "Mod-y", run: () => redo(main), preventDefault: true },
      ])),
      keymap.of(standardKeymap),
      // Save and the formatting keys belong to the editor the table is in.
      // Only those: its other bindings (move line, indent) would act on the
      // table's own lines and break it.
      Prec.lowest(EditorView.domEventHandlers({
        keydown: (e) => {
          if (!(e.metaKey || e.ctrlKey) || e.altKey) return false;
          const key = e.key.toLowerCase();
          if (!["s", "b", "i"].includes(key)) return false;
          // A format key writes marks into the cell, so the cell has to exist.
          if (key !== "s" && !this.materialize()) return false;
          return runScopeHandlers(main, e, "editor");
        },
      })),
      EditorView.domEventHandlers({
        // Images are the editor's to save and link; the main selection follows
        // this one, so the link lands in the cell.
        paste: (e) => {
          if (!this.materialize()) return false;
          return pasteImages(e, main);
        },
        // Edit ▸ Undo is the native menu item, which reaches a focused editor
        // as a history input event rather than a key. This editor keeps no
        // history, so left alone WebKit would undo its DOM edits under it.
        beforeinput: (e) => {
          const command = e.inputType === "historyUndo" ? undo : e.inputType === "historyRedo" ? redo : null;
          if (!command) return false;
          e.preventDefault();
          return command(main);
        },
        blur: () => {
          // Deferred: focus can come straight back (a format command run from
          // the right-click menu refocuses the main editor, which hands it on
          // to this cell again), and a redraw that moved this editor into a
          // new cell element refocuses it a microtask later.
          window.setTimeout(() => this.blurred(), 0);
          return false;
        },
      }),
    ];
  }

  /** The table as it now stands, or null once it is gone. */
  model(): TableModel | null {
    const model = tableModelAt(this.main.state, this.tableFrom);
    return model && model.from === this.tableFrom ? model : null;
  }

  /** The row padding that gives a missing cell its place in the note, and
   *  where the cell's content will start once it is applied. */
  private padding(): { changes: ChangeSpec; base: number } | null {
    const model = this.model();
    const pad = model && padRow(this.main.state, model, this.ref.row);
    if (!pad?.changes) return null;
    const padded = this.main.state.update({ changes: pad.changes }).state;
    const grown = tableModelAt(padded, this.tableFrom);
    const cell = grown && cellOf(grown, this.ref);
    return cell ? { changes: pad.changes, base: cell.from } : null;
  }

  /** The main state as it will be once a missing cell is put in, selection
   *  and all. Null when the cell is not missing, or cannot be put in. */
  prospect(): EditorState | null {
    if (!this.missing) return null;
    const pad = this.padding();
    if (!pad) return null;
    const sel = this.cell.state.selection.main;
    return this.main.state.update({
      changes: pad.changes,
      selection: EditorSelection.single(pad.base + sel.anchor, pad.base + sel.head),
    }).state;
  }

  /** Put a missing cell into the note now, for a command about to act on the
   *  main selection there. False when it cannot be. */
  materialize(): boolean {
    if (!this.missing) return true;
    const pad = this.padding();
    if (!pad) return false;
    this.missing = false;
    this.base = pad.base;
    const sel = this.cell.state.selection.main;
    this.main.dispatch({
      changes: pad.changes,
      selection: EditorSelection.single(pad.base + sel.anchor, pad.base + sel.head),
      annotations: [fromCell.of(true), Transaction.userEvent.of("input")],
    });
    return true;
  }

  /** Replay the cell editor's transactions into the main document. */
  private toMain(trs: readonly Transaction[]) {
    for (const tr of trs) {
      if (tr.annotation(fromMain) || (!tr.docChanged && !tr.selection)) continue;
      // Until something is written, a missing cell has no place in the note
      // for the main selection to follow it to.
      let pad: ChangeSpec | null = null;
      if (this.missing) {
        if (!tr.docChanged) continue;
        const padding = this.padding();
        if (!padding) {
          // Nowhere to put it: close the cell rather than hold text the note
          // does not have.
          queueMicrotask(() => this.end());
          return;
        }
        pad = padding.changes;
        this.missing = false;
        this.base = padding.base;
      }
      const base = this.base;
      const changes: { from: number; to: number; insert: string }[] = [];
      tr.changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
        changes.push({ from: base + fromA, to: base + toA, insert: inserted.toString() });
      });
      const sel = tr.state.selection.main;
      // Not as "select.pointer": the main editor answers that by writing its
      // selection into the DOM, which takes focus from this editor mid-press.
      // A press-drag in a cell then lost the cell to the main editor, which
      // closed it and left the raw row under the caret.
      let event = tr.annotation(Transaction.userEvent);
      if (event?.startsWith("select.pointer")) event = "select";
      const edit: TransactionSpec = {
        changes,
        selection: EditorSelection.single(base + sel.anchor, base + sel.head),
        annotations: [
          fromCell.of(true),
          ...(event ? [Transaction.userEvent.of(event)] : []),
        ],
        sequential: pad !== null,
      };
      // A cell ending in a lone backslash escapes what follows it, and in a
      // cell written tight against its closing pipe ("|a|") that is the pipe:
      // the cell ran on into the next one. A space between them keeps the
      // pipe a pipe, and sits outside the cell's text.
      const end = base + tr.startState.doc.length;
      const before = pad ? this.main.state.update({ changes: pad }).state : this.main.state;
      const guard = endsEscaped(tr.newDoc.toString()) && before.sliceDoc(end, end + 1) === "|"
        ? [{ changes: { from: base + tr.newDoc.length, insert: " " }, sequential: true }]
        : [];
      this.main.dispatch(...(pad ? [{ changes: pad }] : []), edit, ...guard);
    }
  }

  /** Follow a main-editor update the cell editor did not make. */
  followMain(u: ViewUpdate) {
    // Forward past text inserted right at the table's first line (a rewrite
    // from outside adding a paragraph above it). A change that replaces the
    // table from its start maps to that start either way.
    if (u.docChanged) this.tableFrom = u.changes.mapPos(this.tableFrom, 1);
    if (u.transactions.every((tr) => tr.annotation(fromCell))) return;
    if (!u.docChanged && !u.selectionSet) return;
    const model = this.model();
    const sel = u.state.selection.main;
    const ref = model && cellAt(model, sel.from, sel.to);
    // A change from outside that leaves a missing cell still missing, and the
    // main selection where it was parked, leaves the cell open.
    if (this.missing && model && !cellOf(model, this.ref) && !u.selectionSet) return;
    if (!this.missing && model && ref && sameRef(ref, this.ref)) {
      const cell = cellOf(model, ref)!;
      this.base = cell.from;
      const edit = minimalReplacement(this.cell.state.doc.toString(), u.state.sliceDoc(cell.from, cell.to));
      const clamp = (p: number) => Math.min(cell.to, Math.max(cell.from, p)) - cell.from;
      this.cell.dispatch({
        changes: edit ?? [],
        selection: EditorSelection.single(clamp(sel.anchor), clamp(sel.head)),
        annotations: fromMain.of(true),
      });
      return;
    }
    // The selection went somewhere else, an undo of an edit in another cell
    // most likely. Close this one and give the main editor focus, whose own
    // focus handling opens whichever cell the selection is now in.
    const refocus = this.cell.hasFocus;
    this.end();
    // Unless a cell has been opened meanwhile: adding a row from the last
    // cell lands here, then opens the new row's cell straight after.
    if (refocus) queueMicrotask(() => { if (!sessions.has(this.main)) this.main.focus(); });
  }

  private blurred() {
    if (this.ended || this.cell.hasFocus) return;
    // The window lost focus, not the cell: it comes back here on return.
    if (!document.hasFocus() && this.cell.dom.isConnected) return;
    this.end();
  }

  /** Move this editor into the matching cell of a table the main editor
   *  just redrew from scratch. */
  adopt(wrap: HTMLElement) {
    const td = wrap.querySelector<HTMLElement>(cellSelector(this.ref));
    if (!td) {
      queueMicrotask(() => this.end());
      return;
    }
    const focused = this.cell.hasFocus;
    td.replaceChildren(this.cell.dom);
    td.classList.add("editing");
    this.td = td;
    if (focused) queueMicrotask(() => this.cell.focus());
  }

  /** Close the editor and draw the cell's content back in its place. */
  end() {
    if (this.ended) return;
    this.ended = true;
    if (sessions.get(this.main) === this) sessions.delete(this.main);
    this.cell.destroy();
    this.td.classList.remove("editing");
    if (!this.td.isConnected) return;
    this.td.replaceChildren();
    const model = this.model();
    const cell = model && cellOf(model, this.ref);
    if (cell) renderCell(ctxFor(this.main.state), cell, this.td);
  }

  /** Go to another cell of this table, by its position in the grid. */
  private goTo(ref: CellRef, place: "start" | "end" | "all"): boolean {
    const model = this.model();
    if (!model) return false;
    open(this.main, model, ref, place);
    return true;
  }

  /** Leave the table, above it or below it, for the main editor. */
  private leave(down: boolean): boolean {
    const model = this.model();
    if (!model) return false;
    const exit = down ? exitBelow(this.main.state.doc, model) : exitAbove(model);
    if (!exit) return true; // the table opens the document: stay put
    this.end();
    this.main.dispatch(exitSpec(exit));
    this.main.focus();
    return true;
  }

  /** Tab: the next cell, row by row, with its content selected; past the
   *  last cell, a new row. Shift-Tab goes back and stops at the first. */
  private tab(dir: 1 | -1): boolean {
    const model = this.model();
    if (!model) return false;
    let { row, col } = this.ref;
    col += dir;
    if (col >= model.cols) { col = 0; row++; }
    if (col < 0) { col = model.cols - 1; row--; }
    if (row < 0) return true;
    if (row >= gridRows(model).length) return this.addRow(0);
    return this.goTo({ row, col }, "all");
  }

  /** Enter: the cell below; past the last row, a new row. Shift-Enter: the
   *  cell above. A line break has no place in a cell either way. */
  private enter(dir: 1 | -1): boolean {
    const model = this.model();
    if (!model) return false;
    const row = this.ref.row + dir;
    if (row < 0) return true;
    if (row >= gridRows(model).length) return this.addRow(this.ref.col);
    return this.goTo({ row, col: this.ref.col }, "end");
  }

  private addRow(col: number): boolean {
    const model = this.model();
    if (!model) return false;
    const last = gridRows(model).length - 1;
    const lastCell = cellOf(model, { row: last, col: 0 });
    if (!lastCell) return false;
    // editTable reads the cell from the main selection, which mirrors this
    // editor's; point it at the last row first.
    const state = this.main.state.update({ selection: { anchor: lastCell.from } }).state;
    const spec = editTable(state, "rowBelow");
    if (!spec) return false;
    this.main.dispatch(spec);
    const grown = this.model();
    if (!grown) return false;
    open(this.main, grown, { row: last + 1, col }, "start");
    return true;
  }

  /** Up and down leave the cell only from its first or last visual line;
   *  inside a wrapped cell they move between its lines as usual. */
  private vertical(dir: 1 | -1): boolean {
    const view = this.cell;
    const sel = view.state.selection.main;
    if (!sel.empty) return false;
    const here = view.coordsAtPos(sel.head);
    const edge = view.coordsAtPos(dir > 0 ? view.state.doc.length : 0);
    if (here && edge && Math.abs(here.top - edge.top) > 2) return false;
    const model = this.model();
    if (!model) return false;
    const row = this.ref.row + dir;
    if (row < 0 || row >= gridRows(model).length) return this.leave(dir > 0);
    return this.goTo({ row, col: this.ref.col }, dir > 0 ? "start" : "end");
  }

  /** Backspace or Delete at an escaped pipe takes the backslash and the pipe
   *  together, the one character the rendered cell shows. */
  private deletePipe(dir: 1 | -1): boolean {
    const sel = this.cell.state.selection.main;
    if (!sel.empty) return false;
    const range = escapedPipeAt(this.cell.state.doc.toString(), sel.head, dir);
    if (!range) return false;
    this.cell.dispatch({
      changes: range, selection: { anchor: range.from },
      userEvent: dir < 0 ? "delete.backward" : "delete.forward", scrollIntoView: true,
    });
    return true;
  }

  /** Left and right cross into the neighbouring cell from the cell's ends. */
  private horizontal(dir: 1 | -1): boolean {
    const sel = this.cell.state.selection.main;
    if (!sel.empty || sel.head !== (dir > 0 ? this.cell.state.doc.length : 0)) return false;
    const model = this.model();
    if (!model) return false;
    let { row, col } = this.ref;
    col += dir;
    if (col >= model.cols) { col = 0; row++; }
    if (col < 0) { col = model.cols - 1; row--; }
    if (row < 0 || row >= gridRows(model).length) return this.leave(dir > 0);
    return this.goTo({ row, col }, dir > 0 ? "start" : "end");
  }
}

export const exitSpec = (exit: TableExit): TransactionSpec => {
  const spec: TransactionSpec = { selection: { anchor: exit.anchor }, scrollIntoView: true };
  if (!exit.insert) return spec;
  const { from, text } = exit.insert;
  // Lines made for the caret to leave by, not written: no undo step, and
  // taken out again if the caret leaves them unused (exitLines).
  return {
    ...spec,
    changes: { from, insert: text },
    effects: setExitLines.of({ from, to: from + text.length }),
    annotations: Transaction.addToHistory.of(false),
  };
};

/**
 * Open a cell editor on `ref`, or move the open one's selection when it is
 * already there. `place` sets the caret, or `anchor`/`head` do, as main
 * document positions inside the cell's content.
 */
function open(
  main: EditorView, model: TableModel, ref: CellRef,
  place: "start" | "end" | "all", anchor?: number, head?: number,
): CellSession | null {
  // A short row has no cell in this column yet. It is edited all the same,
  // and the row is padded out once something is written in it; until then
  // the main selection waits at the end of the row.
  const row = gridRows(model)[ref.row];
  if (!row || ref.col >= model.cols) return null;
  const cell = cellOf(model, ref);
  if (!cell) {
    anchor = head = row.to;
  } else if (anchor === undefined) {
    anchor = place === "end" ? cell.to : cell.from;
    head = place === "all" ? cell.to : anchor;
  }
  head ??= anchor;
  const current = sessions.get(main);
  if (current && !current.ended && current.tableFrom === model.from && sameRef(current.ref, ref)
    && current.td.isConnected) {
    if (current.missing) {
      current.cell.focus();
      return current;
    }
    const len = current.cell.state.doc.length;
    const off = (p: number) => Math.min(len, Math.max(0, p - current.base));
    current.cell.dispatch({ selection: EditorSelection.single(off(anchor), off(head)) });
    current.cell.focus();
    return current;
  }
  current?.end();
  const td = tableWrap(main, model.from)?.querySelector<HTMLElement>(cellSelector(ref));
  if (!td) return null;
  const session = new CellSession(main, td, model.from, ref, cell, anchor, head);
  sessions.set(main, session);
  session.cell.focus();
  main.dispatch({ selection: EditorSelection.single(anchor, head), annotations: fromCell.of(true) });
  return session;
}

/** A press on a rendered table: follow a link or a tag, or edit the cell. */
function pressTable(view: EditorView, wrap: HTMLElement, e: MouseEvent) {
  const target = e.target as HTMLElement | null;
  const current = sessions.get(view);
  // A press inside the open cell is that cell editor's to handle.
  if (current && target && current.cell.dom.contains(target)) return;
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
  e.preventDefault();
  // The right button's caret is placed as its menu opens (tableCellToPointer).
  if (e.button !== 0) return;
  const el = (target?.closest?.("[data-row]") as HTMLElement | null) ?? rowEndCell(wrap, e.clientY);
  // Read the table now: this element may have been reused for a later
  // version of it, and the closure that registered this handler is stale.
  const model = el && tableModelAt(view.state, view.posAtDOM(wrap));
  if (!el || !model) return;
  const session = open(view, model, refOf(el), "end");
  if (!session) return;
  // Hand the press itself to the cell editor, so a drag from here selects
  // within the cell and a double press picks a word, as anywhere else. Not
  // Shift: there is no selection in this cell yet for it to extend.
  session.cell.contentDOM.dispatchEvent(new MouseEvent("mousedown", {
    bubbles: true, cancelable: true, view: window, detail: e.detail,
    clientX: e.clientX, clientY: e.clientY, screenX: e.screenX, screenY: e.screenY,
    button: e.button, buttons: e.buttons,
    altKey: e.altKey, metaKey: e.metaKey, ctrlKey: e.ctrlKey,
  }));
}

/**
 * For the right-click menu: put the caret where the pointer is when that is
 * on a rendered table, in the cell under it (keeping a selection the press is
 * inside). False when the point is not on a table.
 */
export function tableCellToPointer(view: EditorView, x: number, y: number): boolean {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  const wrap = el?.closest?.(".lp-table-wrap") as HTMLElement | null;
  if (!el || !wrap || !view.contentDOM.contains(wrap)) return false;
  const current = sessions.get(view);
  if (current && current.cell.dom.contains(el)) {
    const pos = current.cell.posAtCoords({ x, y }, false);
    const sel = current.cell.state.selection.main;
    if (pos < sel.from || pos > sel.to) current.cell.dispatch({ selection: { anchor: pos } });
    current.cell.focus();
    return true;
  }
  const cellEl = (el.closest("[data-row]") as HTMLElement | null) ?? rowEndCell(wrap, y);
  const model = cellEl && tableModelAt(view.state, view.posAtDOM(wrap));
  if (!cellEl || !model) return true;
  const session = open(view, model, refOf(cellEl), "end");
  if (session) session.cell.dispatch({ selection: { anchor: session.cell.posAtCoords({ x, y }, false) } });
  return true;
}

/**
 * The state a menu over `view` describes. While the open cell is one its row
 * does not have yet, that is the note with the row padded out and the
 * selection in the new cell, which is what the menu's commands will act on
 * (see prepareCell); the note itself is left alone until one of them runs.
 * Right-clicking such a cell used to pad its row at once, whatever was
 * picked from the menu after, including nothing.
 */
export function menuState(view: EditorView): EditorState {
  const session = sessions.get(view);
  return (session && !session.ended && session.prospect()) || view.state;
}

/** Put the open cell into the note if its row does not have it yet, for a
 *  command about to act on the main selection there. */
export function prepareCell(view: EditorView) {
  const session = sessions.get(view);
  if (session && !session.ended) session.materialize();
}

/** The editor of the cell open in `view`, if one is. */
export function openCellEditor(view: EditorView): EditorView | null {
  const session = sessions.get(view);
  return session && !session.ended ? session.cell : null;
}

/**
 * Hands the main editor's caret to a cell editor whenever it lands in a
 * rendered table: arrowing into one from the line above or below, the right
 * click menu refocusing the editor after a format command, a find match, an
 * undo. The widget covers the table's text, so a caret left there would be
 * invisible and type into the raw row.
 */
const cellFocus = ViewPlugin.fromClass(class {
  constructor(readonly view: EditorView) {}

  update(u: ViewUpdate) {
    sessions.get(u.view)?.followMain(u);
    if (!u.view.hasFocus || !(u.focusChanged || u.selectionSet)) return;
    if (u.transactions.some((tr) => tr.annotation(fromCell))) return;
    const sel = u.state.selection;
    if (sel.ranges.length > 1) return;
    const { anchor, head } = sel.main;
    const model = tableNear(u.state, head);
    if (!model) return;
    const prev = u.selectionSet ? u.startState.selection.main.head : null;
    const pointer = u.transactions.some((tr) => tr.isUserEvent("select.pointer"));
    const entry = tableEntry(model, u.state.doc, anchor, head, prev, pointer);
    if (!entry) return;
    const view = u.view;
    // Not from inside an update: opening a cell dispatches.
    queueMicrotask(() => {
      const now = view.state.selection.main;
      if (now.anchor !== anchor || now.head !== head || !view.hasFocus) return;
      if (entry.kind === "leave") {
        view.dispatch(exitSpec(entry.exit));
        return;
      }
      const current = tableModelAt(view.state, model.from);
      if (current && current.from === model.from) open(view, current, entry.ref, "end", entry.anchor, entry.head);
    });
  }

  destroy() {
    sessions.get(this.view)?.end();
  }
});

/** The column of `row` under the horizontal position `x`, from the drawn
 *  table; the nearest end when `x` is beside it. */
function columnAt(wrap: HTMLElement | null, row: number, x: number): number {
  const cells = wrap ? Array.from(wrap.querySelectorAll("tr")[row]?.children ?? []) : [];
  for (let col = 0; col < cells.length; col++) {
    if (x < cells[col].getBoundingClientRect().right) return col;
  }
  return Math.max(0, cells.length - 1);
}

/**
 * Up and down into a table. CodeMirror's vertical motion steps clean over a
 * block widget, from the line above a table to the line below it, so without
 * this the keyboard had no way into one: when the motion would cross a
 * rendered table, the caret enters it instead, in the column under it.
 */
function arrowIntoTable(view: EditorView, dir: 1 | -1): boolean {
  const sel = view.state.selection.main;
  if (!sel.empty) return false;
  const next = view.moveVertically(sel, dir > 0).head;
  let crossed: TableModel | null = null;
  for (let node = syntaxTree(view.state).topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== "Table") continue;
    const ahead = dir > 0 ? node.from > sel.head && node.from <= next : node.to < sel.head && node.to >= next;
    if (!ahead) continue;
    crossed = tableModel(view.state, node);
    if (dir > 0) break; // the first one below; upward, the last one above
  }
  if (!crossed) return false;
  const row = dir > 0 ? 0 : gridRows(crossed).length - 1;
  const x = view.coordsAtPos(sel.head)?.left ?? 0;
  const col = Math.min(columnAt(tableWrap(view, crossed.from), row, x), crossed.cols - 1);
  return open(view, crossed, { row, col }, dir > 0 ? "start" : "end") !== null;
}

/**
 * Backspace at the start of the line under a table would join that line onto
 * the table's last row, where it reads as one cell too many: GFM drops it, so
 * the text vanished from view. Step into the last cell instead. An empty line
 * goes with it, unless what follows would then sit straight under the table,
 * which would make it a row.
 */
const tableKeys = Prec.high(keymap.of([{
  key: "ArrowDown", run: (view) => arrowIntoTable(view, 1),
}, {
  key: "ArrowUp", run: (view) => arrowIntoTable(view, -1),
}, {
  key: "Backspace",
  run: (view) => {
    const sel = view.state.selection.main;
    if (!sel.empty) return false;
    const line = view.state.doc.lineAt(sel.head);
    if (sel.head !== line.from || line.number === 1) return false;
    const model = tableModelAt(view.state, line.from - 1, -1);
    if (!model || model.to !== line.from - 1) return false;
    const after = line.number < view.state.doc.lines ? view.state.doc.line(line.number + 1) : null;
    if (line.length === 0 && (!after || after.text.trim() === "")) {
      view.dispatch({ changes: { from: line.from - 1, to: line.from }, userEvent: "delete.backward" });
    }
    const rows = gridRows(model);
    const last = rows.length - 1;
    open(view, model, { row: last, col: Math.min(rows[last].cells.length, model.cols) - 1 }, "end");
    return true;
  },
}]));

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

const sameShape = (a: TableModel, b: TableModel) =>
  (a.header === null) === (b.header === null) && a.rows.length === b.rows.length && a.cols === b.cols;

class TableWidget extends WidgetType {
  constructor(
    readonly model: TableModel,
    readonly index: DocsIndex | null,
    readonly cross: CrossRefs | null,
    /** A selection runs across the table (not inside one cell of it). */
    readonly selected: boolean,
  ) { super(); }

  eq(other: TableWidget) {
    // The index and the cross-refs matter too: they decide whether a wikilink
    // in a cell draws as resolved.
    return other.model.text === this.model.text && other.selected === this.selected
      && other.index === this.index && other.cross === this.cross;
  }

  ignoreEvent() { return true; } // the table, and the cell editor in it, own their events

  get estimatedHeight() { return 30 * (this.model.rows.length + 1) + 10; }

  /** Fill one cell element with its content and alignment. */
  private drawCell(ctx: CellCtx, el: HTMLElement, ref: CellRef) {
    el.style.textAlign = this.model.align[ref.col] ?? "";
    el.replaceChildren();
    const cell = cellOf(this.model, ref);
    if (cell) renderCell(ctx, cell, el);
  }

  toDOM(view: EditorView) {
    const model = this.model;
    const ctx = ctxFor(view.state);
    const wrap = document.createElement("div");
    wrap.className = `lp-table-wrap${this.selected ? " selected" : ""}`;
    wrap.setAttribute("contenteditable", "false");
    const table = document.createElement("table");
    table.className = "lp-table";
    wrap.appendChild(table);

    const drawRow = (row: number, tag: "th" | "td"): HTMLTableRowElement => {
      const tr = document.createElement("tr");
      for (let col = 0; col < model.cols; col++) {
        const el = document.createElement(tag);
        // A short row's missing cells are drawn too, and take a press: the
        // row is padded out when one of them is edited.
        el.dataset.row = String(row);
        el.dataset.col = String(col);
        this.drawCell(ctx, el, { row, col });
        tr.appendChild(el);
      }
      return tr;
    };

    let row = 0;
    if (model.header) {
      const thead = document.createElement("thead");
      thead.appendChild(drawRow(row++, "th"));
      table.appendChild(thead);
    }
    const tbody = document.createElement("tbody");
    for (let i = 0; i < model.rows.length; i++) tbody.appendChild(drawRow(row++, "td"));
    table.appendChild(tbody);

    wrap.addEventListener("mousedown", (e) => pressTable(view, wrap, e));

    // Redrawn while a cell in it is being edited: keep the editor.
    const session = sessions.get(view);
    if (session && !session.ended && session.tableFrom === model.from) session.adopt(wrap);
    return wrap;
  }

  // Typing in a cell changes the table's text on every keystroke. Redrawing
  // the table each time would throw away the cell editor in it, focus and
  // all, so a table whose shape held is patched cell by cell instead, around
  // the cell being edited.
  updateDOM(dom: HTMLElement, view: EditorView, prev: TableWidget): boolean {
    const session = sessions.get(view);
    const live = session && !session.ended ? session : null;
    const holds = live ? dom.contains(live.td) : false;
    // Never patch one table's element into another's while it holds the open
    // editor, and never let the open editor's table take an element without it.
    if (live && holds !== (live.tableFrom === this.model.from)) return false;
    if (!sameShape(prev.model, this.model)) return false;
    const ctx = ctxFor(view.state);
    for (const el of Array.from(dom.querySelectorAll<HTMLElement>("[data-row]"))) {
      if (live && el === live.td) {
        el.style.textAlign = this.model.align[Number(el.dataset.col)] ?? "";
        continue;
      }
      this.drawCell(ctx, el, refOf(el));
    }
    dom.classList.toggle("selected", this.selected);
    return true;
  }
}

// ── The field ────────────────────────────────────────────────────────────────

interface TableState {
  decos: DecorationSet;
  /** The main selection sits inside one table cell. */
  inCell: boolean;
}

function buildTables(state: EditorState): TableState {
  const index = state.facet(docsIndexFacet);
  const cross = state.facet(crossRefsFacet);
  const decos: Range<Decoration>[] = [];
  const ranges = state.selection.ranges;
  let inCell = false;
  // Only top-level tables get a widget (tableModel rejects the nested ones
  // anyway), so the walk stays over the document's own children rather than
  // descending the whole tree on every keystroke.
  for (let node = syntaxTree(state).topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== "Table") continue;
    const model = tableModel(state, node);
    if (!model) continue;
    let selected = false;
    for (const r of ranges) {
      if (r.to < model.from || r.from > model.to) continue;
      if (cellAt(model, r.from, r.to)) inCell = true;
      else if (!r.empty) selected = true;
    }
    decos.push(Decoration.replace({
      widget: new TableWidget(model, index, cross, selected), block: true,
    }).range(model.from, model.to));
  }
  return { decos: Decoration.set(decos, true), inCell };
}

const tableField = StateField.define<TableState>({
  create: buildTables,
  update(value, tr) {
    // The parse runs in the background on a long document, so a table below
    // the first chunk only becomes a node some transactions later.
    const treeGrew = syntaxTree(tr.startState) !== syntaxTree(tr.state);
    const refsChanged =
      tr.startState.facet(docsIndexFacet) !== tr.state.facet(docsIndexFacet) ||
      tr.startState.facet(crossRefsFacet) !== tr.state.facet(crossRefsFacet);
    if (!tr.docChanged && !tr.selection && !treeGrew && !refsChanged) return value;
    return buildTables(tr.state);
  },
  provide: (f) => [
    EditorView.decorations.from(f, (v) => v.decos),
    // While the selection is inside a cell, the cell editor draws it. The
    // main editor's own layers would paint it over the whole table, which
    // its text sits under.
    EditorView.editorAttributes.from(f, (v): Record<string, string> => (v.inCell ? { class: "lp-in-cell" } : {})),
  ],
});

// ── Lines made to leave a table by ───────────────────────────────────────────

interface LineRange {
  from: number;
  to: number;
}

const setExitLines = StateEffect.define<LineRange | null>();

/**
 * The blank lines added under a table that ends the note, for as long as
 * nothing has been written on them. A note has no position below its last
 * line, so leaving such a table downward has to make one; but arrowing out of
 * it, or clicking under it, then edited the note, which autosaved the change
 * for a keypress that wrote nothing. Lines left unused are taken back out.
 */
export const exitLines = StateField.define<LineRange | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setExitLines)) return e.value;
    if (!value || !tr.docChanged) return value;
    // Anything written on them makes them the user's. An insertion right at
    // their start is the table's last cell growing, not that.
    let touched = false;
    tr.changes.iterChangedRanges((fromA, toA) => {
      if (toA > value.from && fromA <= value.to) touched = true;
    });
    return touched ? null : { from: tr.changes.mapPos(value.from, 1), to: tr.changes.mapPos(value.to, -1) };
  },
});

/** True for a transaction that only adds or takes out exit lines: nothing
 *  the user wrote, so nothing to save or report as a change. */
export function exitLinesOnly(tr: Transaction): boolean {
  return tr.effects.some((e) => e.is(setExitLines));
}

/** The note as written: the document without exit lines nobody has used.
 *  Saving them saved a change to a note the user had only arrowed through. */
export function writtenText(state: EditorState): string {
  const lines = state.field(exitLines, false);
  const doc = state.doc;
  return lines ? doc.sliceString(0, lines.from) + doc.sliceString(lines.to) : doc.toString();
}

/** Takes the unused exit lines out when a selection change leaves them. */
const dropExitLines = EditorState.transactionFilter.of((tr) => {
  const lines = tr.startState.field(exitLines, false);
  if (!lines || tr.docChanged || !tr.selection) return tr;
  if (tr.selection.ranges.every((r) => r.from >= lines.from && r.to <= lines.to)) return tr;
  return [tr, {
    changes: { from: lines.from, to: lines.to },
    effects: setExitLines.of(null),
    // The transaction was a selection change only, so this keeps it out of
    // the history whole: the lines never went in.
    annotations: Transaction.addToHistory.of(false),
    sequential: true,
  }];
});

/**
 * Rendered GFM tables for the docs note editor and the issue description,
 * edited a cell at a time without leaving the rendered view.
 *
 * Deliberately not atomic, unlike the frontmatter card: arrowing left or
 * right onto the widget has to land the main editor's caret at its edge, which
 * is what cellFocus hands on to the first or last cell. Atomic ranges would hop
 * the whole table. (Up and down hop it regardless; tableKeys handles those.)
 */
export const markdownTables: Extension = [tableField, exitLines, dropExitLines, cellFocus, tableKeys];
