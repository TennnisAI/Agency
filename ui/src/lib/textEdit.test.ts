import { describe, expect, it } from "vitest";
import { minimalReplacement } from "./textEdit";

/** Apply a replacement the way CodeMirror would, to prove it reconstructs. */
const apply = (text: string, r: ReturnType<typeof minimalReplacement>) =>
  r === null ? text : text.slice(0, r.from) + r.insert + text.slice(r.to);

describe("minimalReplacement", () => {
  it("is null for identical text", () => {
    expect(minimalReplacement("same", "same")).toBeNull();
    expect(minimalReplacement("", "")).toBeNull();
  });

  it("touches only the middle when the ends are shared", () => {
    expect(minimalReplacement("head MID tail", "head NEW tail")).toEqual({
      from: 5, to: 8, insert: "NEW",
    });
  });

  it("appends without rewriting the note above it", () => {
    const before = "# Note\n\nbody\n";
    const after = "# Note\n\nbody\nmore\n";
    expect(minimalReplacement(before, after)).toEqual({
      from: before.length, to: before.length, insert: "more\n",
    });
  });

  it("deletes as an empty insert", () => {
    expect(minimalReplacement("a bcd e", "a e")).toEqual({ from: 2, to: 6, insert: "" });
  });

  it("handles an empty side", () => {
    expect(minimalReplacement("", "text")).toEqual({ from: 0, to: 0, insert: "text" });
    expect(minimalReplacement("text", "")).toEqual({ from: 0, to: 4, insert: "" });
  });

  it("never crosses the prefix it already matched", () => {
    // "aaa" -> "aaaaa": prefix and suffix both want the same characters.
    const r = minimalReplacement("aaa", "aaaaa")!;
    expect(r.from).toBeLessThanOrEqual(r.to);
    expect(apply("aaa", r)).toBe("aaaaa");
  });

  it("reconstructs for assorted pairs", () => {
    const pairs: [string, string][] = [
      ["", "x"],
      ["x", ""],
      ["abc", "abd"],
      ["line1\nline2\n", "line1\ninserted\nline2\n"],
      ["---\nkey: a\n---\nbody", "---\nkey: b\n---\nbody"],
      ["repeat repeat", "repeat"],
      ["one", "two"],
    ];
    for (const [before, after] of pairs) {
      expect(apply(before, minimalReplacement(before, after))).toBe(after);
    }
  });
});
