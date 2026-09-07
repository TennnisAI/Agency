import { describe, expect, it } from "vitest";
import { hasConflicts, parseConflicts, resolveAll, resolveBlock } from "./conflictFile";

// The shape git leaves behind, written out rather than built from a helper: the
// exact bytes are what this module is about.
const ONE = [
  "one",
  "<<<<<<< HEAD",
  "TWO-main",
  "=======",
  "TWO-feat",
  ">>>>>>> agent/feature",
  "three",
  "",
].join("\n");

const TWO = [
  "<<<<<<< HEAD",
  "a-main",
  "=======",
  "a-feat",
  ">>>>>>> agent/feature",
  "middle",
  "<<<<<<< HEAD",
  "b-main",
  "=======",
  "b-feat",
  ">>>>>>> agent/feature",
  "",
].join("\n");

describe("parseConflicts", () => {
  it("reads both sides and the branch names off the markers", () => {
    const [b] = parseConflicts(ONE);
    expect(b.current).toEqual(["TWO-main"]);
    expect(b.incoming).toEqual(["TWO-feat"]);
    expect(b.currentLabel).toBe("HEAD");
    expect(b.incomingLabel).toBe("agent/feature");
  });

  it("drops the base section of a diff3 conflict, which is neither side", () => {
    const diff3 = [
      "<<<<<<< HEAD",
      "mine",
      "||||||| merged common ancestors",
      "original",
      "=======",
      "theirs",
      ">>>>>>> other",
      "",
    ].join("\n");
    const [b] = parseConflicts(diff3);
    expect(b.current).toEqual(["mine"]);
    expect(b.incoming).toEqual(["theirs"]);
    expect(resolveBlock(diff3, 0, "both")).toBe("mine\ntheirs\n");
  });

  it("finds every block in a file that has several", () => {
    expect(parseConflicts(TWO).map((b) => b.incoming)).toEqual([["a-feat"], ["b-feat"]]);
  });

  it("ignores a row of equals signs in a file with no conflict in it", () => {
    const doc = "Heading\n=======\ntext\n>>>>>>> quoted\n";
    expect(parseConflicts(doc)).toEqual([]);
    expect(hasConflicts(doc)).toBe(false);
  });

  it("refuses a block it cannot read rather than guessing at one", () => {
    // Truncated (no `=======`), and nested — both leave the file alone, which
    // is the only safe answer when the buttons this feeds rewrite it.
    expect(parseConflicts("<<<<<<< HEAD\nmine\nthe end\n")).toEqual([]);
    expect(parseConflicts("<<<<<<< HEAD\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> b\n")).toEqual([]);
  });
});

describe("resolveBlock", () => {
  it("keeps one side and takes every marker with it", () => {
    expect(resolveBlock(ONE, 0, "current")).toBe("one\nTWO-main\nthree\n");
    expect(resolveBlock(ONE, 0, "incoming")).toBe("one\nTWO-feat\nthree\n");
    expect(resolveBlock(ONE, 0, "both")).toBe("one\nTWO-main\nTWO-feat\nthree\n");
  });

  it("leaves the file's other conflicts exactly as git wrote them", () => {
    const once = resolveBlock(TWO, 0, "current");
    expect(once).toBe("a-main\nmiddle\n<<<<<<< HEAD\nb-main\n=======\nb-feat\n>>>>>>> agent/feature\n");
    // And the second block is still resolvable, at its new index.
    expect(resolveBlock(once, 0, "incoming")).toBe("a-main\nmiddle\nb-feat\n");
  });

  it("is a no-op on an index that is not a block", () => {
    expect(resolveBlock(ONE, 4, "current")).toBe(ONE);
    expect(resolveBlock("no conflict here\n", 0, "current")).toBe("no conflict here\n");
  });

  it("keeps an empty side empty rather than leaving a marker behind", () => {
    // The whole file was one conflict and the incoming side deleted it: the
    // answer is an empty file, not a file holding the leftover markers.
    const deleted = "<<<<<<< HEAD\nkept\n=======\n>>>>>>> other\n";
    expect(resolveBlock(deleted, 0, "incoming")).toBe("");
    expect(hasConflicts(resolveBlock(deleted, 0, "incoming"))).toBe(false);
  });
});

describe("resolveAll", () => {
  it("takes the same side everywhere and leaves nothing unmerged", () => {
    expect(resolveAll(TWO, "incoming")).toBe("a-feat\nmiddle\nb-feat\n");
    expect(hasConflicts(resolveAll(TWO, "current"))).toBe(false);
  });

  it("returns a file with no conflicts in it byte for byte", () => {
    const plain = "line\n\n  indented\r\nwith \\ backslashes\n";
    expect(resolveAll(plain, "current")).toBe(plain);
  });
});
