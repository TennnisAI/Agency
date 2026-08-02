import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { syntaxTree } from "@codemirror/language";
import { docsMarkdown } from "./livePreview";

/** Node names covering `pos`, innermost last. */
function nodesAt(doc: string, pos: number): string[] {
  const state = EditorState.create({ doc, extensions: [docsMarkdown()] });
  const names: string[] = [];
  for (let n = syntaxTree(state).resolveInner(pos, 1); n; n = n.parent!) {
    names.push(n.name);
    if (!n.parent) break;
  }
  return names;
}

describe("docsMarkdown", () => {
  it("parses ==highlight== as its own node", () => {
    expect(nodesAt("a ==marked== b", 5)).toContain("Highlight");
  });

  it("does not highlight a lone == pair across lines", () => {
    expect(nodesAt("a ==open\nclose== b", 5)).not.toContain("Highlight");
  });

  it("leaves a bare = alone", () => {
    expect(nodesAt("x = y", 4)).not.toContain("Highlight");
  });

  it("still parses emphasis and wikilinks", () => {
    expect(nodesAt("a **bold** b", 6)).toContain("StrongEmphasis");
    expect(nodesAt("a [[Note]] b", 6)).toContain("Wikilink");
  });
});
