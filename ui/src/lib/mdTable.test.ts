import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { docsMarkdown } from "./livePreview";
import { parseAlign, tableModelAt, TableModel } from "./mdTable";

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

  it("trims the padding off cell ranges", () => {
    const m = model("|   a   | b |\n|---|---|\n| 1 | 2 |")!;
    expect(m.header!.cells[0]).toEqual({ from: 4, to: 5 });
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
