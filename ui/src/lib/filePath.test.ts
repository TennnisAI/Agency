import { describe, it, expect } from "vitest";
import { ancestorDirs, joinPath } from "./filePath";

describe("joinPath", () => {
  it("joins under the root with no leading slash", () => {
    expect(joinPath("", "src")).toBe("src");
  });
  it("joins nested segments with a single slash", () => {
    expect(joinPath("src", "main.rs")).toBe("src/main.rs");
    expect(joinPath("a/b", "c")).toBe("a/b/c");
  });
});

describe("ancestorDirs", () => {
  it("lists the folders on the way to a file, outermost first", () => {
    expect(ancestorDirs("a/b/c.ts")).toEqual(["a", "a/b"]);
  });
  it("has nothing to open for a top-level file", () => {
    expect(ancestorDirs("README.md")).toEqual([]);
  });
  it("leaves the leaf off, so revealing a folder must append it", () => {
    expect(ancestorDirs("src/term")).toEqual(["src"]);
  });
  it("ignores a trailing slash rather than emitting an empty segment", () => {
    expect(ancestorDirs("src/term/")).toEqual(["src"]);
  });
});
