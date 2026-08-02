import { ChangeSpec, EditorSelection, EditorState, TransactionSpec } from "@codemirror/state";
import { Command } from "@codemirror/view";

// Markdown formatting edits for the Docs editor: the transforms behind the
// right-click Format / Paragraph / Insert menus (and the Mod-b / Mod-i keys).
//
// Every transform is a pure state -> TransactionSpec function so the behaviour
// is testable without a DOM, and returns null when there is nothing to do.

// ── Inline marks ─────────────────────────────────────────────────────────────

export type InlineMark = "bold" | "italic" | "strike" | "highlight" | "code";

const INLINE: Record<InlineMark, string> = {
  bold: "**",
  italic: "*",
  strike: "~~",
  highlight: "==",
  code: "`",
};

/**
 * Toggle an inline mark around the selection (or, with no selection, around the
 * word under the cursor — an empty spot just gets the pair with the cursor
 * parked between them).
 *
 * Removal handles both shapes the selection can take: marks sitting just
 * outside it (`**|bold|**`) and marks inside it (`|**bold**|`).
 */
export function toggleInline(state: EditorState, kind: InlineMark): TransactionSpec {
  const mark = INLINE[kind];
  const n = mark.length;
  const doc = state.doc;
  let { from, to } = state.selection.main;
  if (from === to) {
    const word = state.wordAt(from);
    if (word) {
      from = word.from;
      to = word.to;
    }
  }
  const slice = (a: number, b: number) =>
    a < 0 || b > doc.length ? "" : doc.sliceString(a, b);

  // A single "*" of a "**bold**" pair is not an italic mark — toggling italic
  // on bold text must nest (***both***), not peel one star off each side.
  const boldClash = kind === "italic" && slice(from - 2, from) === "**" && slice(to, to + 2) === "**";
  if (!boldClash && slice(from - n, from) === mark && slice(to, to + n) === mark) {
    return {
      changes: [{ from: from - n, to: from }, { from: to, to: to + n }],
      selection: EditorSelection.range(from - n, to - n),
    };
  }

  const inner = doc.sliceString(from, to);
  const innerClash = kind === "italic" && inner.startsWith("**") && inner.endsWith("**");
  if (!innerClash && inner.length >= 2 * n && inner.startsWith(mark) && inner.endsWith(mark)) {
    return {
      changes: [{ from, to: from + n }, { from: to - n, to }],
      selection: EditorSelection.range(from, to - 2 * n),
    };
  }

  return {
    changes: [{ from, insert: mark }, { from: to, insert: mark }],
    selection: from === to
      ? EditorSelection.cursor(from + n)
      : EditorSelection.range(from + n, to + n),
  };
}

const MARK_CHARS = new Set(["*", "~", "=", "`"]);

/**
 * Strip inline emphasis marks from the selection (or the word under the
 * cursor). The range grows outward over adjacent mark characters first, so
 * clearing from inside `**bold**` takes the stars with it. Underscores are left
 * alone — they are far more often part of a word than emphasis.
 */
export function clearFormatting(state: EditorState): TransactionSpec | null {
  const doc = state.doc;
  let { from, to } = state.selection.main;
  if (from === to) {
    const word = state.wordAt(from);
    if (!word) return null;
    from = word.from;
    to = word.to;
  }
  while (from > 0 && MARK_CHARS.has(doc.sliceString(from - 1, from))) from--;
  while (to < doc.length && MARK_CHARS.has(doc.sliceString(to, to + 1))) to++;
  const text = doc.sliceString(from, to);
  const cleaned = text.replace(/\*+|~~|==|`+/g, "");
  if (cleaned === text) return null;
  return {
    changes: { from, to, insert: cleaned },
    selection: EditorSelection.range(from, from + cleaned.length),
  };
}

// ── Block structure ──────────────────────────────────────────────────────────

export type ListKind = "bullet" | "ordered" | "task";

/** A line split into the prefixes markdown block syntax stacks up front. */
export interface LineParts {
  indent: string;
  quote: string; // one "> " per nesting level, verbatim
  list: string; // "- ", "1. ", "- [ ] ", …
  heading: number; // 0 = not a heading
  text: string;
}

const LIST_RE = /^(?:[-*+][ \t]+(?:\[[ xX]\][ \t]+)?|\d+[.)][ \t]+)/;

export function splitLine(text: string): LineParts {
  let rest = text;
  const indent = /^[ \t]*/.exec(rest)![0];
  rest = rest.slice(indent.length);
  let quote = "";
  while (rest.startsWith(">")) {
    const m = /^>[ \t]?/.exec(rest)![0];
    quote += m;
    rest = rest.slice(m.length);
  }
  const list = LIST_RE.exec(rest)?.[0] ?? "";
  rest = rest.slice(list.length);
  const heading = /^(#{1,6})[ \t]+/.exec(rest);
  if (heading) rest = rest.slice(heading[0].length);
  return { indent, quote, list, heading: heading ? heading[1].length : 0, text: rest };
}

export function joinLine(p: LineParts): string {
  const head = p.heading ? `${"#".repeat(p.heading)} ` : "";
  return p.indent + p.quote + p.list + head + p.text;
}

export function listKind(list: string): ListKind | null {
  if (!list) return null;
  if (/^\d/.test(list)) return "ordered";
  if (list.includes("[")) return "task";
  return "bullet";
}

/** What the line holding the cursor currently is — drives the menu checkmarks. */
export interface BlockState {
  heading: number;
  list: ListKind | null;
  quote: boolean;
}

export function blockState(state: EditorState): BlockState {
  const p = splitLine(state.doc.lineAt(state.selection.main.head).text);
  return { heading: p.heading, list: listKind(p.list), quote: p.quote !== "" };
}

/** Rewrite every line the selection touches. */
function mapLines(
  state: EditorState,
  f: (p: LineParts, i: number) => LineParts,
): TransactionSpec | null {
  const doc = state.doc;
  const { from, to } = state.selection.main;
  const first = doc.lineAt(from).number;
  const last = doc.lineAt(to).number;
  const changes: ChangeSpec[] = [];
  for (let n = first; n <= last; n++) {
    const line = doc.line(n);
    const next = joinLine(f(splitLine(line.text), n - first));
    if (next !== line.text) changes.push({ from: line.from, to: line.to, insert: next });
  }
  return changes.length ? { changes } : null;
}

/** Every line the selection touches, as parts. */
function selectedParts(state: EditorState): LineParts[] {
  const doc = state.doc;
  const { from, to } = state.selection.main;
  const out: LineParts[] = [];
  for (let n = doc.lineAt(from).number, last = doc.lineAt(to).number; n <= last; n++) {
    out.push(splitLine(doc.line(n).text));
  }
  return out;
}

/** Set (not toggle) the heading level; 0 turns the line back into body text. */
export function setHeading(state: EditorState, level: number): TransactionSpec | null {
  return mapLines(state, (p) => ({ ...p, heading: level }));
}

/** Toggle the selection into a list of `kind` — or out of it, if it already is one. */
export function toggleList(state: EditorState, kind: ListKind): TransactionSpec | null {
  const off = selectedParts(state).every((p) => listKind(p.list) === kind);
  let ordinal = 0;
  return mapLines(state, (p) => {
    if (off) return { ...p, list: "" };
    const marker = kind === "bullet" ? "- " : kind === "task" ? "- [ ] " : `${++ordinal}. `;
    return { ...p, list: marker };
  });
}

/** Toggle blockquote (`> `) on the selected lines; unquoting drops all levels. */
export function toggleQuote(state: EditorState): TransactionSpec | null {
  const off = selectedParts(state).every((p) => p.quote !== "");
  return mapLines(state, (p) => ({ ...p, quote: off ? "" : p.quote || "> " }));
}

// ── Insertions ───────────────────────────────────────────────────────────────

export type InsertKind = "wikilink" | "link" | "table" | "callout" | "rule" | "codeblock";

/** Block templates: the text, plus the span to select once it lands. */
const BLOCKS: Record<"table" | "callout" | "rule" | "codeblock", { text: string; at: number; len: number }> = {
  table: { text: "| Column | Column |\n| --- | --- |\n|  |  |", at: 2, len: 6 },
  callout: { text: "> [!note] Title\n> ", at: 10, len: 5 },
  rule: { text: "---", at: 3, len: 0 },
  codeblock: { text: "```\n\n```", at: 4, len: 0 },
};

/**
 * Insert a construct at the cursor: links wrap the selection inline, blocks go
 * on a line of their own (reusing the current line when it is blank).
 */
export function insertConstruct(state: EditorState, kind: InsertKind): TransactionSpec {
  const doc = state.doc;
  const { from, to } = state.selection.main;

  if (kind === "wikilink" || kind === "link") {
    const sel = doc.sliceString(from, to);
    if (kind === "wikilink") {
      return {
        changes: { from, to, insert: `[[${sel}]]` },
        // Cursor inside the brackets, where the completion popup wants it.
        selection: EditorSelection.cursor(from + 2 + sel.length),
      };
    }
    const insert = `[${sel}](url)`;
    const urlAt = from + sel.length + 3;
    return {
      changes: { from, to, insert },
      selection: EditorSelection.range(urlAt, urlAt + 3),
    };
  }

  const block = BLOCKS[kind];
  const line = doc.lineAt(to);
  const blank = line.text.trim() === "";
  const start = blank ? line.from : line.to + 1;
  const at = start + block.at;
  return {
    changes: blank
      ? { from: line.from, to: line.to, insert: block.text }
      : { from: line.to, insert: `\n${block.text}` },
    selection: block.len ? EditorSelection.range(at, at + block.len) : EditorSelection.cursor(at),
  };
}

// ── Commands ─────────────────────────────────────────────────────────────────

/** Lift a transform into a CodeMirror command (for the keymap or a menu item). */
export function formatCommand(f: (state: EditorState) => TransactionSpec | null): Command {
  return (view) => {
    const spec = f(view.state);
    if (spec) view.dispatch(spec);
    return true;
  };
}
