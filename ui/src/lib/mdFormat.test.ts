import { describe, expect, it } from "vitest";
import { EditorState, TransactionSpec } from "@codemirror/state";
import {
  blockState, clearFormatting, insertConstruct, joinLine, setHeading, splitLine,
  toggleInline, toggleList, toggleQuote,
} from "./mdFormat";

// Test docs mark the selection with "|": one for a cursor, two for a range.
function mk(spec: string): EditorState {
  const marks: number[] = [];
  let doc = "";
  for (const ch of spec) {
    if (ch === "|") marks.push(doc.length);
    else doc += ch;
  }
  const [anchor = 0, head = marks[0] ?? 0] = marks;
  return EditorState.create({ doc, selection: { anchor, head } });
}

/** Apply a spec and render the result back into the "|" notation. */
function out(state: EditorState, spec: TransactionSpec | null): string {
  const next = spec ? state.update(spec).state : state;
  const { from, to } = next.selection.main;
  const text = next.doc.toString();
  return from === to
    ? `${text.slice(0, from)}|${text.slice(from)}`
    : `${text.slice(0, from)}|${text.slice(from, to)}|${text.slice(to)}`;
}

const text = (state: EditorState, spec: TransactionSpec | null) =>
  (spec ? state.update(spec).state : state).doc.toString();

describe("toggleInline", () => {
  it("wraps the selection", () => {
    const s = mk("hello |world|");
    expect(out(s, toggleInline(s, "bold"))).toBe("hello **|world|**");
  });

  it("unwraps marks sitting outside the selection", () => {
    const s = mk("**|bold|**");
    expect(out(s, toggleInline(s, "bold"))).toBe("|bold|");
  });

  it("unwraps marks inside the selection", () => {
    const s = mk("|**bold**|");
    expect(out(s, toggleInline(s, "bold"))).toBe("|bold|");
  });

  it("nests italic inside bold instead of peeling one star off", () => {
    const s = mk("**|bold|**");
    expect(text(s, toggleInline(s, "italic"))).toBe("***bold***");
  });

  it("nests italic when the selection includes the bold marks", () => {
    const s = mk("|**bold**|");
    expect(text(s, toggleInline(s, "italic"))).toBe("***bold***");
  });

  it("expands a bare cursor to the word under it", () => {
    const s = mk("hello wo|rld");
    expect(text(s, toggleInline(s, "code"))).toBe("hello `world`");
  });

  it("un-marks the word under the cursor", () => {
    const s = mk("hello ~~wo|rld~~");
    expect(text(s, toggleInline(s, "strike"))).toBe("hello world");
  });

  it("drops an empty pair at the cursor when there is no word", () => {
    const s = mk("hello |");
    expect(out(s, toggleInline(s, "highlight"))).toBe("hello ==|==");
  });
});

describe("clearFormatting", () => {
  it("strips marks around the word under the cursor", () => {
    const s = mk("a **bo|ld** b");
    expect(text(s, clearFormatting(s))).toBe("a bold b");
  });

  it("strips every mark inside a selection", () => {
    const s = mk("|**bold** and `code` and ==hi==|");
    expect(text(s, clearFormatting(s))).toBe("bold and code and hi");
  });

  it("leaves underscores alone", () => {
    const s = mk("|snake_case|");
    expect(clearFormatting(s)).toBeNull();
  });

  it("does nothing on unformatted text", () => {
    const s = mk("pl|ain");
    expect(clearFormatting(s)).toBeNull();
  });
});

describe("splitLine", () => {
  it("splits a task item", () => {
    expect(splitLine("  - [ ] task")).toEqual({
      indent: "  ", quote: "", list: "- [ ] ", heading: 0, text: "task",
    });
  });

  it("splits a quoted heading", () => {
    expect(splitLine("> ## Heading")).toEqual({
      indent: "", quote: "> ", list: "", heading: 2, text: "Heading",
    });
  });

  it("splits nested quotes and an ordered item", () => {
    expect(splitLine(">> 3. item")).toEqual({
      indent: "", quote: ">> ", list: "3. ", heading: 0, text: "item",
    });
  });

  it("does not read a bare # as a heading", () => {
    expect(splitLine("#tag here").heading).toBe(0);
  });

  it("round-trips through joinLine", () => {
    for (const line of ["  - [x] done", "> ### Deep", "1) first", "plain", ""]) {
      expect(joinLine(splitLine(line))).toBe(line);
    }
  });
});

describe("setHeading", () => {
  it("promotes body text", () => {
    const s = mk("ti|tle");
    expect(text(s, setHeading(s, 1))).toBe("# title");
  });

  it("replaces an existing level", () => {
    const s = mk("### ti|tle");
    expect(text(s, setHeading(s, 2))).toBe("## title");
  });

  it("drops back to body at level 0", () => {
    const s = mk("## ti|tle");
    expect(text(s, setHeading(s, 0))).toBe("title");
  });

  it("keeps the list marker of a list item", () => {
    const s = mk("- it|em");
    expect(text(s, setHeading(s, 2))).toBe("- ## item");
  });

  it("applies to every line the selection touches", () => {
    const s = mk("|one\ntwo|");
    expect(text(s, setHeading(s, 1))).toBe("# one\n# two");
  });

  it("does nothing when the level already matches", () => {
    const s = mk("# ti|tle");
    expect(setHeading(s, 1)).toBeNull();
  });
});

describe("toggleList", () => {
  it("bullets the selected lines", () => {
    const s = mk("|one\ntwo|");
    expect(text(s, toggleList(s, "bullet"))).toBe("- one\n- two");
  });

  it("numbers the selected lines in order", () => {
    const s = mk("|one\ntwo\nthree|");
    expect(text(s, toggleList(s, "ordered"))).toBe("1. one\n2. two\n3. three");
  });

  it("turns a list off when every line already matches", () => {
    const s = mk("|- one\n- two|");
    expect(text(s, toggleList(s, "bullet"))).toBe("one\ntwo");
  });

  it("converts between list kinds", () => {
    const s = mk("- it|em");
    expect(text(s, toggleList(s, "task"))).toBe("- [ ] item");
  });

  it("treats a checked task as a task", () => {
    const s = mk("- [x] do|ne");
    expect(text(s, toggleList(s, "task"))).toBe("done");
  });

  it("keeps indentation and heading marks", () => {
    const s = mk("  ## ti|tle");
    expect(text(s, toggleList(s, "bullet"))).toBe("  - ## title");
  });
});

describe("toggleQuote", () => {
  it("quotes the selected lines", () => {
    const s = mk("|one\ntwo|");
    expect(text(s, toggleQuote(s))).toBe("> one\n> two");
  });

  it("unquotes when every line is quoted, dropping all levels", () => {
    const s = mk("|> one\n>> two|");
    expect(text(s, toggleQuote(s))).toBe("one\ntwo");
  });

  it("quotes the rest when only some lines are quoted", () => {
    const s = mk("|> one\ntwo|");
    expect(text(s, toggleQuote(s))).toBe("> one\n> two");
  });
});

describe("insertConstruct", () => {
  it("wraps the selection in a wikilink, cursor inside", () => {
    const s = mk("see |Note|");
    expect(out(s, insertConstruct(s, "wikilink"))).toBe("see [[Note|]]");
  });

  it("wraps the selection in a link with the url selected", () => {
    const s = mk("see |Note|");
    expect(out(s, insertConstruct(s, "link"))).toBe("see [Note](|url|)");
  });

  it("reuses a blank line for a block", () => {
    const s = mk("intro\n|");
    expect(text(s, insertConstruct(s, "rule"))).toBe("intro\n---");
  });

  it("puts a block on a new line below a non-empty one", () => {
    const s = mk("in|tro");
    expect(text(s, insertConstruct(s, "codeblock"))).toBe("intro\n```\n\n```");
  });

  it("lands the cursor inside a new code block", () => {
    const s = mk("in|tro");
    expect(out(s, insertConstruct(s, "codeblock"))).toBe("intro\n```\n|\n```");
  });

  it("selects the first column header of a new table", () => {
    const s = mk("|");
    expect(out(s, insertConstruct(s, "table")))
      .toBe("| |Column| | Column |\n| --- | --- |\n|  |  |");
  });

  it("selects the callout title", () => {
    const s = mk("|");
    expect(out(s, insertConstruct(s, "callout"))).toBe("> [!note] |Title|\n> ");
  });
});

describe("blockState", () => {
  it("reports the line under the cursor", () => {
    expect(blockState(mk("- [ ] ta|sk"))).toEqual({ heading: 0, list: "task", quote: false });
    expect(blockState(mk("> ## ti|tle"))).toEqual({ heading: 2, list: null, quote: true });
    expect(blockState(mk("pl|ain"))).toEqual({ heading: 0, list: null, quote: false });
  });
});
