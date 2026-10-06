import { describe, expect, it } from "vitest";
import { EditorState, Text } from "@codemirror/state";
import { history, undoDepth } from "@codemirror/commands";
import { docsMarkdown } from "./livePreview";
import {
  cellAt, cellInput, cellOf, cellRepairs, cellSafe, cellSafeAtSelection, editTable, endsEscaped,
  escapedPipeAt, exitBelow, exitLines, exitLinesOnly, exitSpec, formatTable, markdownTables, padRow,
  parseAlign, TableEdit, tableEdits, tableEntry, tableModelAt, TableModel, writtenText,
} from "./mdTable";

function model(doc: string, pos = 0): TableModel | null {
  const state = EditorState.create({ doc, extensions: [docsMarkdown()] });
  return tableModelAt(state, pos);
}

/** Cell text per row, header first — what the widget would draw. */
function grid(doc: string, pos = 0): string[][] | null {
  const state = EditorState.create({ doc, extensions: [docsMarkdown()] });
  const m = tableModelAt(state, pos);
  if (!m) return null;
  const cells = (row: { to: number; cells: { from: number; to: number }[] }) =>
    Array.from({ length: m.cols }, (_, c) => {
      const cell = row.cells[c];
      return cell ? state.doc.sliceString(cell.from, cell.to) : "";
    });
  return [...(m.header ? [cells(m.header)] : []), ...m.rows.map(cells)];
}

describe("parseAlign", () => {
  it("reads the colons", () => {
    expect(parseAlign("|---|:--|:-:|--:|")).toEqual([null, "left", "center", "right"]);
  });

  it("works without the outer pipes", () => {
    expect(parseAlign("--- | :-:")).toEqual([null, "center"]);
  });

  it("handles a single column", () => {
    expect(parseAlign("|---|")).toEqual([null]);
    expect(parseAlign("---")).toEqual([null]);
  });
});

describe("tableModelAt", () => {
  it("reads a table's rows and columns", () => {
    expect(grid("| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |")).toEqual([
      ["a", "b"], ["1", "2"], ["3", "4"],
    ]);
  });

  it("covers the whole table's line range", () => {
    const doc = "intro\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nafter";
    const m = model(doc, doc.indexOf("| 1"))!;
    expect(doc.slice(m.from, m.to)).toBe("| a | b |\n|---|---|\n| 1 | 2 |");
    expect(m.text).toBe(doc.slice(m.from, m.to));
  });

  it("keeps an empty cell in its column", () => {
    // The parser emits no TableCell for an empty cell, so counting nodes
    // would slide "c" one column to the left.
    expect(grid("| a |  | c |\n|---|---|---|\n| 1 |  | 3 |")).toEqual([
      ["a", "", "c"], ["1", "", "3"],
    ]);
  });

  it("pads a short row and drops a long row's extra cells", () => {
    expect(grid("| a | b |\n|---|---|\n| 1 |\n| 1 | 2 | 3 |")).toEqual([
      ["a", "b"], ["1", ""], ["1", "2"],
    ]);
  });

  it("works without the outer pipes", () => {
    expect(grid("a | b\n--- | ---\n1 | 2")).toEqual([["a", "b"], ["1", "2"]]);
  });

  it("keeps an escaped pipe inside its cell", () => {
    expect(grid("| a | b |\n|---|---|\n| x \\| y | 2 |")).toEqual([
      ["a", "b"], ["x \\| y", "2"],
    ]);
  });

  it("trims the padding off cell ranges, and keeps the span between the pipes", () => {
    const m = model("|   a   | b |\n|---|---|\n| 1 | 2 |")!;
    expect(m.header!.cells[0]).toEqual({ from: 4, to: 5, start: 1, end: 8 });
  });

  it("puts an empty cell's caret one space in", () => {
    // So typing into "|  |" makes "| x |".
    const m = model("| a |  |\n|---|---|")!;
    expect(m.header!.cells[1]).toEqual({ from: 6, to: 6, start: 5, end: 7 });
  });

  it("carries the column alignments", () => {
    expect(model("| a | b |\n|:--|--:|\n| 1 | 2 |")!.align).toEqual(["left", "right"]);
  });

  it("renders a header-only table", () => {
    expect(grid("| a | b |\n|---|---|")).toEqual([["a", "b"]]);
  });

  it("refuses a table that does not start at a line start", () => {
    // A block widget has to replace whole lines; a table inside a blockquote
    // begins after the quote mark.
    const doc = "> | a | b |\n> |---|---|\n> | 1 | 2 |";
    expect(model(doc, doc.indexOf("a"))).toBe(null);
  });

  it("refuses a table nested in a list item", () => {
    const doc = "- | a | b |\n  |---|---|\n  | 1 | 2 |";
    expect(model(doc, doc.indexOf("a"))).toBe(null);
  });

  it("returns null outside any table", () => {
    expect(model("just a paragraph\n\n| a |\n|---|", 3)).toBe(null);
  });
});

/** A state holding `doc`, its selection at the `^` markers (one for a caret,
 *  two for a range), which are removed. */
function stateAt(marked: string): EditorState {
  const first = marked.indexOf("^");
  const second = marked.indexOf("^", first + 1);
  const doc = marked.replace(/\^/g, "");
  const anchor = first;
  const head = second < 0 ? first : second - 1;
  return EditorState.create({ doc, selection: { anchor, head }, extensions: [docsMarkdown()] });
}

/** The document after `edit`, with the caret marked "^". */
function edited(marked: string, edit: TableEdit): string | null {
  const state = stateAt(marked);
  const spec = editTable(state, edit);
  if (!spec) return null;
  const next = state.update(spec).state;
  const at = next.selection.main.head;
  return next.doc.sliceString(0, at) + "^" + next.doc.sliceString(at);
}

describe("cellAt", () => {
  const doc = "| a | bb |\n|---|---|\n| 1 |  |";
  const m = model(doc)!;

  it("finds the cell a range sits in, padding included", () => {
    expect(cellAt(m, doc.indexOf("bb"))).toEqual({ row: 0, col: 1 });
    expect(cellAt(m, doc.indexOf("bb") - 1, doc.indexOf("bb") + 3)).toEqual({ row: 0, col: 1 });
    expect(cellAt(m, doc.indexOf("1"))).toEqual({ row: 1, col: 0 });
  });

  it("finds an empty cell", () => {
    expect(cellAt(m, doc.length - 2)).toEqual({ row: 1, col: 1 });
  });

  it("has no cell for a range across two, the separator row, or outside a pipe", () => {
    expect(cellAt(m, doc.indexOf("a"), doc.indexOf("bb"))).toBe(null);
    expect(cellAt(m, doc.indexOf("---"))).toBe(null);
    expect(cellAt(m, 0)).toBe(null);
  });

  it("names cells by row, header first", () => {
    expect(cellOf(m, { row: 1, col: 0 })).toMatchObject({ from: doc.indexOf("1") });
    expect(cellOf(m, { row: 2, col: 0 })).toBe(null);
    expect(cellOf(m, { row: 0, col: 2 })).toBe(null);
  });
});

describe("tableEntry", () => {
  const doc = "above\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nbelow";
  const text = Text.of(doc.split("\n"));
  const m = model(doc, doc.indexOf("| a"))!;

  it("hands a selection inside a cell to that cell", () => {
    const at = doc.indexOf("b |");
    expect(tableEntry(m, text, at, at + 1, null, false))
      .toEqual({ kind: "cell", ref: { row: 0, col: 1 }, anchor: at, head: at + 1 });
  });

  it("pulls a caret on a cell's padding onto its text", () => {
    const at = doc.indexOf("b |");
    expect(tableEntry(m, text, at - 1, at - 1, null, false))
      .toEqual({ kind: "cell", ref: { row: 0, col: 1 }, anchor: at, head: at });
  });

  it("enters the first cell arrowing down, the last cell arrowing up", () => {
    expect(tableEntry(m, text, m.from, m.from, 2, false))
      .toEqual({ kind: "cell", ref: { row: 0, col: 0 }, anchor: m.from + 2, head: m.from + 2 });
    expect(tableEntry(m, text, m.to, m.to, doc.length, false))
      .toEqual({ kind: "cell", ref: { row: 1, col: 1 }, anchor: m.to - 2, head: m.to - 2 });
  });

  it("leaves the table for a click in the margin beside it", () => {
    expect(tableEntry(m, text, m.to, m.to, 0, true))
      .toEqual({ kind: "leave", exit: { anchor: doc.indexOf("below") } });
    expect(tableEntry(m, text, m.from, m.from, 0, true))
      .toEqual({ kind: "leave", exit: { anchor: m.from - 1 } });
  });

  it("leaves a table whose last row has no closing pipe for a click under it", () => {
    const open = "| h |\n|---|\n| 1"; // the click lands on the last cell's text
    const state = EditorState.create({ doc: open, extensions: [docsMarkdown()] });
    const t = tableModelAt(state, 0)!;
    expect(tableEntry(t, state.doc, t.to, t.to, 0, true)?.kind).toBe("leave");
    // From the keyboard, the same position is that cell.
    expect(tableEntry(t, state.doc, t.to, t.to, 0, false)?.kind).toBe("cell");
  });

  it("leaves a range across the table's structure alone", () => {
    expect(tableEntry(m, text, doc.indexOf("a"), doc.indexOf("2"), null, false)).toBe(null);
  });

  it("ignores a selection outside the table", () => {
    expect(tableEntry(m, text, 1, 1, null, false)).toBe(null);
    expect(tableEntry(m, text, 1, doc.indexOf("2"), null, false)).toBe(null);
  });
});

describe("exitBelow", () => {
  const exit = (doc: string) => {
    const state = EditorState.create({ doc, extensions: [docsMarkdown()] });
    const e = exitBelow(state.doc, tableModelAt(state, 0)!);
    const next = state.update({
      ...(e.insert ? { changes: { from: e.insert.from, insert: e.insert.text } } : {}),
      selection: { anchor: e.anchor },
    }).state;
    const at = next.selection.main.head;
    return next.doc.sliceString(0, at) + "^" + next.doc.sliceString(at);
  };

  it("makes a blank line and a line to write on when the table ends the note", () => {
    // Straight under the last row, a plain line would be read as another row.
    expect(exit("| a |\n|---|")).toBe("| a |\n|---|\n\n^");
    expect(exit("| a |\n|---|\n")).toBe("| a |\n|---|\n\n^");
  });

  it("goes past the blank line to what follows", () => {
    expect(exit("| a |\n|---|\n\nnext")).toBe("| a |\n|---|\n\n^next");
    expect(exit("| a |\n|---|\n\n\n")).toBe("| a |\n|---|\n\n^\n");
  });

  it("stops at a block that ended the table by itself", () => {
    expect(exit("| a |\n|---|\n# head")).toBe("| a |\n|---|\n^# head");
  });
});

describe("cellSafe", () => {
  it("escapes a pipe, which would end the cell", () => {
    expect(cellSafe("a|b", "")).toBe("a\\|b");
    expect(cellSafe("||", "x")).toBe("\\|\\|");
  });

  it("leaves a pipe already escaped, in the text or just before it", () => {
    expect(cellSafe("a\\|b", "")).toBe("a\\|b");
    expect(cellSafe("|", "\\")).toBe("|");
  });

  it("pairs backslashes the way the table parser does", () => {
    // "\\" is an escaped backslash, so the pipe after it is bare.
    expect(cellSafe("|", "a\\\\")).toBe("\\|");
    expect(cellSafe("|", "a\\\\\\")).toBe("|");
    expect(cellSafe("\\\\|", "")).toBe("\\\\\\|");
  });

  it("turns line breaks into spaces", () => {
    expect(cellSafe("one\ntwo\r\nthree", "")).toBe("one two three");
  });

  it("applies only when the selection is in a cell", () => {
    expect(cellSafeAtSelection(stateAt("| a^ |\n|---|"), "x|y")).toBe("x\\|y");
    expect(cellSafeAtSelection(stateAt("prose^\n\n| a |\n|---|"), "x|y")).toBe("x|y");
  });
});

describe("editTable", () => {
  const doc = "| a | b |\n| --- | :-: |\n| 1 | 2 |\n| 3 | 4 |";

  it("adds a row below a body row, caret in the same column", () => {
    expect(edited(doc.replace("2 |", "2^ |"), "rowBelow"))
      .toBe("| a | b |\n| --- | :-: |\n| 1 | 2 |\n|  | ^ |\n| 3 | 4 |");
  });

  it("adds a row below the header under the separator", () => {
    expect(edited(doc.replace("a |", "a^ |"), "rowBelow"))
      .toBe("| a | b |\n| --- | :-: |\n| ^ |  |\n| 1 | 2 |\n| 3 | 4 |");
  });

  it("adds a row above a body row, and none above the header", () => {
    expect(edited(doc.replace("3 |", "3^ |"), "rowAbove"))
      .toBe("| a | b |\n| --- | :-: |\n| 1 | 2 |\n| ^ |  |\n| 3 | 4 |");
    expect(edited(doc.replace("a |", "a^ |"), "rowAbove")).toBe(null);
  });

  it("deletes a row, caret into the row that took its place", () => {
    expect(edited(doc.replace("2 |", "2^ |"), "deleteRow"))
      .toBe("| a | b |\n| --- | :-: |\n| 3 | ^4 |");
    expect(edited(doc.replace("4 |", "4^ |"), "deleteRow"))
      .toBe("| a | b |\n| --- | :-: |\n| 1 | ^2 |");
    expect(edited(doc.replace("a |", "a^ |"), "deleteRow")).toBe(null);
  });

  it("adds a column, keeping the alignments and lining the pipes up", () => {
    expect(edited(doc.replace("1 |", "1^ |"), "colAfter")).toBe([
      "| a   |     | b   |",
      "| --- | --- | :-: |",
      "| 1   | ^    | 2   |",
      "| 3   |     | 4   |",
    ].join("\n"));
    expect(edited(doc.replace("b |", "b^ |"), "colBefore")).toBe([
      "| a   | ^    | b   |",
      "| --- | --- | :-: |",
      "| 1   |     | 2   |",
      "| 3   |     | 4   |",
    ].join("\n"));
  });

  it("deletes a column, and not the last one", () => {
    expect(edited(doc.replace("a |", "a^ |"), "deleteCol")).toBe([
      "| b^   |",
      "| :-: |",
      "| 2   |",
      "| 4   |",
    ].join("\n"));
    expect(edited("| a^ |\n|---|\n| 1 |", "deleteCol")).toBe(null);
  });

  it("keeps a short row's cells, padding it out", () => {
    expect(edited("| a | b |\n|---|---|\n| 1^ |", "colAfter")).toBe([
      "| a   |     | b   |",
      "| --- | --- | --- |",
      "| 1   | ^    |     |",
    ].join("\n"));
  });

  it("offers nothing outside a cell", () => {
    expect(tableEdits(stateAt("^prose\n\n| a |\n|---|"))).toBe(null);
    expect(edited("^prose\n\n| a |\n|---|", "rowBelow")).toBe(null);
  });
});

describe("formatTable", () => {
  it("pads columns to their widest cell and draws each alignment", () => {
    expect(formatTable([["name", "n"], ["x", "12345"]], ["left", "right"]).text).toBe([
      "| name | n     |",
      "| :--- | ----: |",
      "| x    | 12345 |",
    ].join("\n"));
  });
});

describe("padRow", () => {
  it("gives a short row the cells it is missing", () => {
    const state = stateAt("| a | b | c |\n|---|---|---|\n| 1 |^");
    const m = tableModelAt(state, 0)!;
    expect(state.update(padRow(state, m, 1)!).state.doc.line(3).text).toBe("| 1 |  |  |");
  });

  it("closes a row that has no trailing pipe first", () => {
    const state = stateAt("a | b\n--- | ---\n1^");
    const m = tableModelAt(state, 0)!;
    expect(state.update(padRow(state, m, 1)!).state.doc.line(3).text).toBe("1 |  |");
  });

  it("closes a row that ends in an escaped pipe", () => {
    const state = stateAt("| a | b |\n|---|---|\n| x \\|^");
    const spec = padRow(state, tableModelAt(state, 0)!, 1)!;
    const next = state.update(spec).state;
    expect(next.doc.line(3).text).toBe("| x \\| |  |");
    expect(grid(next.doc.toString())).toEqual([["a", "b"], ["x \\|", ""]]);
  });

  it("leaves a full row alone", () => {
    const state = stateAt("| a |\n|---|\n| 1 |^");
    expect(padRow(state, tableModelAt(state, 0)!, 1)).toBe(null);
  });
});

describe("endsEscaped", () => {
  it("is true for an unpaired trailing backslash only", () => {
    expect(endsEscaped("a\\")).toBe(true);
    expect(endsEscaped("a\\\\")).toBe(false);
    expect(endsEscaped("a\\\\\\")).toBe(true);
    expect(endsEscaped("a")).toBe(false);
    expect(endsEscaped("")).toBe(false);
  });
});

describe("cellRepairs", () => {
  it("escapes bare pipes and flattens line breaks, counting a CRLF once", () => {
    expect(cellRepairs("a|b\r\nc")).toEqual([
      { from: 1, to: 1, insert: "\\" },
      { from: 3, to: 5, insert: " " },
    ]);
  });

  it("finds nothing in text a cell can hold", () => {
    expect(cellRepairs("a \\| b [[x\\|y]]")).toEqual([]);
  });
});

describe("cellInput", () => {
  const cell = (doc: string, caret: number) =>
    EditorState.create({ doc, selection: { anchor: caret }, extensions: [cellInput] });

  it("puts back the escape a backspace took from a pipe, caret in front of it", () => {
    const state = cell("a \\| b", 3).update({ changes: { from: 2, to: 3 }, selection: { anchor: 2 } }).state;
    expect(state.doc.toString()).toBe("a \\| b");
    expect(state.selection.main.head).toBe(2);
  });

  it("re-escapes a pipe a typed backslash would free", () => {
    const state = cell("a\\|", 1).update({ changes: { from: 1, insert: "\\" }, selection: { anchor: 2 } }).state;
    expect(state.doc.toString()).toBe("a\\\\\\|");
    expect(state.selection.main.head).toBe(2);
  });

  it("escapes a typed pipe, caret after it", () => {
    const state = cell("ab", 1).update({ changes: { from: 1, insert: "|" }, selection: { anchor: 2 } }).state;
    expect(state.doc.toString()).toBe("a\\|b");
    expect(state.selection.main.head).toBe(3);
  });
});

describe("exitLines", () => {
  const doc = "| h |\n|---|\n| 1 |";
  const start = () => EditorState.create({ doc, extensions: [docsMarkdown(), markdownTables, history()] });
  const leave = (state: EditorState) => state.update(exitSpec(exitBelow(state.doc, tableModelAt(state, 0)!))).state;

  it("adds lines to leave a table that ends the note by, outside the history", () => {
    const state = leave(start());
    expect(state.doc.toString()).toBe(doc + "\n\n");
    expect(state.selection.main.head).toBe(doc.length + 2);
    expect(state.field(exitLines)).toEqual({ from: doc.length, to: doc.length + 2 });
    expect(undoDepth(state)).toBe(0);
  });

  it("takes them back out when the caret leaves them unused", () => {
    const state = leave(start()).update({ selection: { anchor: 2 } }).state;
    expect(state.doc.toString()).toBe(doc);
    expect(state.selection.main.head).toBe(2);
    expect(state.field(exitLines)).toBe(null);
    expect(undoDepth(state)).toBe(0);
  });

  it("keeps them once something is written there", () => {
    let state = leave(start());
    state = state.update({ changes: { from: state.doc.length, insert: "x" }, selection: { anchor: state.doc.length + 1 } }).state;
    state = state.update({ selection: { anchor: 2 } }).state;
    expect(state.doc.toString()).toBe(doc + "\n\nx");
    expect(state.field(exitLines)).toBe(null);
  });

  it("leaves them out of the note as written, and marks their transactions", () => {
    const s0 = start();
    const tr = s0.update(exitSpec(exitBelow(s0.doc, tableModelAt(s0, 0)!)));
    expect(exitLinesOnly(tr)).toBe(true);
    expect(writtenText(tr.state)).toBe(doc);
    const back = tr.state.update({ selection: { anchor: 2 } });
    expect(exitLinesOnly(back)).toBe(true);
    expect(exitLinesOnly(s0.update({ changes: { from: 0, insert: "x" } }))).toBe(false);
  });

  it("keeps them while the caret moves between them", () => {
    const state = leave(start()).update({ selection: { anchor: doc.length + 1 } }).state;
    expect(state.doc.toString()).toBe(doc + "\n\n");
  });
});

describe("escapedPipeAt", () => {
  const t = "a \\| b"; // a, space, backslash, pipe, space, b

  it("takes the pair a Backspace after it would split", () => {
    expect(escapedPipeAt(t, 4, -1)).toEqual({ from: 2, to: 4 });
  });

  it("takes the pair a Delete before it would split", () => {
    expect(escapedPipeAt(t, 2, 1)).toEqual({ from: 2, to: 4 });
  });

  it("takes the pair from between its halves, either way", () => {
    expect(escapedPipeAt(t, 3, -1)).toEqual({ from: 2, to: 4 });
    expect(escapedPipeAt(t, 3, 1)).toEqual({ from: 2, to: 4 });
  });

  it("leaves ordinary text, and a backslash that is itself escaped, alone", () => {
    expect(escapedPipeAt(t, 1, -1)).toBe(null);
    expect(escapedPipeAt(t, 5, -1)).toBe(null);
    expect(escapedPipeAt("\\\\x", 2, -1)).toBe(null);
  });
});
