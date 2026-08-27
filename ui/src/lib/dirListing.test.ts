import { describe, expect, it } from "vitest";
import { DirEntry } from "../api";
import { sameListing, visibleDirs } from "./dirListing";

const file = (name: string): DirEntry => ({ name, isDir: false, hasChildren: false });
const dir = (name: string, hasChildren = false): DirEntry => ({ name, isDir: true, hasChildren });

describe("sameListing", () => {
  it("matches identical listings", () => {
    expect(sameListing([dir("src", true), file("a.ts")], [dir("src", true), file("a.ts")])).toBe(true);
    expect(sameListing([], [])).toBe(true);
  });

  it("notices an added, removed or renamed entry", () => {
    expect(sameListing([file("a.ts")], [file("a.ts"), file("b.ts")])).toBe(false);
    expect(sameListing([file("a.ts"), file("b.ts")], [file("a.ts")])).toBe(false);
    expect(sameListing([file("a.ts")], [file("b.ts")])).toBe(false);
  });

  it("notices a folder that gained its first child", () => {
    expect(sameListing([dir("src")], [dir("src", true)])).toBe(false);
  });

  it("notices a name that changed kind", () => {
    expect(sameListing([file("build")], [dir("build")])).toBe(false);
  });
});

describe("visibleDirs", () => {
  const cache = new Map<string, DirEntry[]>([
    ["", [dir("src", true), dir("docs", true), file("README.md")]],
    ["src", [dir("deep", true), file("main.ts")]],
    ["src/deep", [file("leaf.ts")]],
    ["docs", [file("guide.md")]],
  ]);

  it("is just the root when nothing is expanded", () => {
    expect(visibleDirs(cache, new Set())).toEqual([""]);
  });

  it("walks into expanded folders, depth first", () => {
    expect(visibleDirs(cache, new Set(["src", "src/deep"]))).toEqual(["", "src", "src/deep"]);
  });

  it("leaves out an expanded folder whose parent is collapsed", () => {
    expect(visibleDirs(cache, new Set(["src/deep"]))).toEqual([""]);
  });

  it("ignores an expanded folder that has not been listed yet", () => {
    expect(visibleDirs(new Map(), new Set(["src"]))).toEqual([""]);
  });
});
