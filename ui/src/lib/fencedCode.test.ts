import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { docsMarkdown, fencedCodeAt } from "./livePreview";

/** What the code block's copy button would put on the clipboard, given a
 *  caret position (the widget passes the opening fence line's start). */
function copied(doc: string, pos: number): string | null {
  return fencedCodeAt(EditorState.create({ doc, extensions: [docsMarkdown()] }), pos);
}

describe("fencedCodeAt", () => {
  it("returns the body without the fences", () => {
    const doc = "intro\n\n```rust\nfn main() {}\nlet x = 1;\n```\n\nafter";
    expect(copied(doc, doc.indexOf("```rust"))).toBe("fn main() {}\nlet x = 1;");
  });

  it("finds the block from a position inside the code", () => {
    const doc = "```js\nconst a = 1;\n```";
    expect(copied(doc, doc.indexOf("const"))).toBe("const a = 1;");
  });

  it("keeps blank lines and indentation inside the body", () => {
    const doc = "```py\ndef f():\n\n    return 1\n```";
    expect(copied(doc, 0)).toBe("def f():\n\n    return 1");
  });

  it("runs to the end of an unterminated block", () => {
    const doc = "```sh\necho hi\necho bye";
    expect(copied(doc, 0)).toBe("echo hi\necho bye");
  });

  it("is empty for a block with no body", () => {
    expect(copied("```\n```", 0)).toBe("");
    expect(copied("```ts", 0)).toBe("");
  });

  it("handles ~~~ fences", () => {
    expect(copied("~~~\nplain\n~~~", 0)).toBe("plain");
  });

  it("does not treat an inner ``` as the block's end", () => {
    const doc = "````md\n```\nnested\n```\n````";
    expect(copied(doc, 0)).toBe("```\nnested\n```");
  });

  it("returns null outside any fenced block", () => {
    const doc = "just a paragraph\n\n```js\nx\n```";
    expect(copied(doc, 3)).toBe(null);
  });
});
