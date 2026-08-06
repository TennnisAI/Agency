import { describe, expect, it } from "vitest";
import {
  EMPTY_FIND_QUERY, FindQuery, findMatches, queryInvalid, replaceAllIn, replacementFor, searchText,
  wholeWordAt,
} from "./find";

const q = (patch: Partial<FindQuery>): FindQuery => ({ ...EMPTY_FIND_QUERY, ...patch });
const spans = (text: string, query: FindQuery) =>
  findMatches(text, query).map((m) => text.slice(m.from, m.to));

describe("findMatches", () => {
  it("is case-insensitive by default", () => {
    expect(findMatches("Cat cat CAT", q({ search: "cat" }))).toHaveLength(3);
  });

  it("honours match case", () => {
    expect(findMatches("Cat cat CAT", q({ search: "cat", caseSensitive: true }))).toHaveLength(1);
  });

  it("treats a literal query as text, not a pattern", () => {
    expect(spans("a.b axb", q({ search: "a.b" }))).toEqual(["a.b"]);
  });

  it("matches regular expressions when asked", () => {
    expect(spans("a1 b22 c333", q({ search: "[a-z]\\d+", regexp: true })))
      .toEqual(["a1", "b22", "c333"]);
  });

  it("reports no matches for a regexp that doesn't compile", () => {
    expect(findMatches("anything", q({ search: "(unclosed", regexp: true }))).toEqual([]);
  });

  it("keeps whole-word matches only", () => {
    expect(findMatches("in inside pin in.", q({ search: "in", wholeWord: true }))).toHaveLength(2);
  });

  it("counts a match hemmed in by punctuation as a whole word", () => {
    expect(wholeWordAt("(in)", 1, 3)).toBe(true);
    expect(wholeWordAt("pin", 1, 3)).toBe(false);
  });

  it("advances past zero-length matches instead of spinning", () => {
    // `b*` matches empty at every position; the guard is what makes this return.
    expect(findMatches("abc", q({ search: "b*", regexp: true })).length).toBe(4);
  });

  it("stops at the cap", () => {
    expect(findMatches("x".repeat(50), q({ search: "x" }), 10)).toHaveLength(10);
  });

  it("resolves escapes in a literal query, the way CodeMirror does", () => {
    expect(searchText(q({ search: "a\\nb" }))).toBe("a\nb");
    expect(findMatches("a\nb", q({ search: "a\\nb" }))).toHaveLength(1);
  });
});

describe("queryInvalid", () => {
  it("only flags a regexp that can't compile", () => {
    expect(queryInvalid(q({ search: "(", regexp: true }))).toBe(true);
    expect(queryInvalid(q({ search: "(", regexp: false }))).toBe(false);
    expect(queryInvalid(q({ search: "", regexp: true }))).toBe(false);
  });
});

describe("replacementFor", () => {
  it("inserts a literal replacement verbatim, dollars and all", () => {
    const query = q({ search: "cost", replace: "$5" });
    expect(replacementFor("cost", { from: 0, to: 4 }, query)).toBe("$5");
  });

  it("expands capture groups for a regexp replacement", () => {
    const query = q({ search: "(\\w+)@(\\w+)", replace: "$2/$1", regexp: true });
    expect(replacementFor("nic@agency", { from: 0, to: 10 }, query)).toBe("agency/nic");
  });
});

describe("replaceAllIn", () => {
  it("replaces every match and reports the count", () => {
    expect(replaceAllIn("one two one", q({ search: "one", replace: "1" })))
      .toEqual({ text: "1 two 1", count: 2 });
  });

  it("leaves the text alone when nothing matches", () => {
    expect(replaceAllIn("one", q({ search: "zzz", replace: "1" })))
      .toEqual({ text: "one", count: 0 });
  });

  it("handles replacements that change length", () => {
    expect(replaceAllIn("a a a", q({ search: "a", replace: "long" })).text).toBe("long long long");
  });

  it("is not capped — replace all means all", () => {
    const many = "x ".repeat(600);
    expect(replaceAllIn(many, q({ search: "x", replace: "y" })).count).toBe(600);
  });
});
