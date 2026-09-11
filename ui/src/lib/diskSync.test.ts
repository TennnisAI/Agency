import { describe, expect, it } from "vitest";
import { mayHaveChanged, minimalChange, statKey } from "./diskSync";

const at = (mtimeMs: number, size: number) => ({ mtimeMs, size });

describe("mayHaveChanged", () => {
  it("is quiet while mtime and size both hold", () => {
    expect(mayHaveChanged(at(1000, 10), at(1000, 10))).toBe(false);
  });

  it("notices a same-size rewrite by its mtime", () => {
    expect(mayHaveChanged(at(1000, 10), at(1001, 10))).toBe(true);
  });

  it("notices a same-mtime copy by its size", () => {
    expect(mayHaveChanged(at(1000, 10), at(1000, 11))).toBe(true);
  });

  it("re-reads when nothing has been seen yet", () => {
    expect(mayHaveChanged(null, at(1000, 10))).toBe(true);
  });

  it("re-reads when the platform has no mtime to offer", () => {
    expect(mayHaveChanged(at(0, 10), at(0, 10))).toBe(true);
    expect(mayHaveChanged(at(1000, 10), at(0, 10))).toBe(true);
  });

  it("keys on both fields", () => {
    expect(statKey(at(1000, 10))).toBe("1000:10");
  });
});

// Every case checks the contract that matters: applying the change to `from`
// yields `to`, and the change is as narrow as a single range can be.
const apply = (from: string, c: { from: number; to: number; insert: string }) =>
  from.slice(0, c.from) + c.insert + from.slice(c.to);

describe("minimalChange", () => {
  it("is null for identical text", () => {
    expect(minimalChange("abc", "abc")).toBeNull();
    expect(minimalChange("", "")).toBeNull();
  });

  it("replaces one hunk in the middle and leaves the rest alone", () => {
    const from = "line 1\nline 2\nline 3\n";
    const to = "line 1\nline two\nline 3\n";
    const c = minimalChange(from, to)!;
    expect(c).toEqual({ from: 12, to: 13, insert: "two" });
    expect(apply(from, c)).toBe(to);
  });

  it("handles a pure insertion and a pure deletion", () => {
    const ins = minimalChange("ab", "aXb")!;
    expect(ins).toEqual({ from: 1, to: 1, insert: "X" });
    const del = minimalChange("aXb", "ab")!;
    expect(del).toEqual({ from: 1, to: 2, insert: "" });
  });

  it("does not let the suffix reach back into the prefix", () => {
    // "ab" -> "aab": the shared "a" must count once, or the span goes negative.
    const c = minimalChange("ab", "aab")!;
    expect(c.from).toBeLessThanOrEqual(c.to);
    expect(apply("ab", c)).toBe("aab");
    const d = minimalChange("aab", "ab")!;
    expect(d.from).toBeLessThanOrEqual(d.to);
    expect(apply("aab", d)).toBe("ab");
  });

  it("covers appends, prepends, and a total rewrite", () => {
    for (const [from, to] of [
      ["abc", "abcdef"], ["abc", "xyzabc"], ["abc", "xyz"], ["", "abc"], ["abc", ""],
    ]) {
      const c = minimalChange(from, to)!;
      expect(apply(from, c)).toBe(to);
    }
  });

  it("never splits a surrogate pair", () => {
    // Both texts share the high surrogate of one emoji, differ in the low one.
    const from = "a\u{1F600}b"; // 😀
    const to = "a\u{1F601}b"; // 😁
    const c = minimalChange(from, to)!;
    expect(c).toEqual({ from: 1, to: 3, insert: "\u{1F601}" });
    expect(apply(from, c)).toBe(to);
    // And when the tail would stop between the halves.
    const from2 = "\u{1F600}\u{1F600}";
    const to2 = "\u{1F601}\u{1F600}";
    const c2 = minimalChange(from2, to2)!;
    expect(c2.insert).not.toMatch(/^[\uDC00-\uDFFF]/);
    expect(apply(from2, c2)).toBe(to2);
  });
});
