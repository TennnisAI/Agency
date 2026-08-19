import { describe, expect, it } from "vitest";
import { DocFile } from "../api";
import { buildIndex } from "./docsIndex";
import { allDirPaths, buildDocsTree } from "./docsTree";

const doc = (path: string, text: string): DocFile => ({ path, text, tooLarge: false });

describe("buildDocsTree", () => {
  it("nests notes under a folder per path segment", () => {
    const tree = buildDocsTree(
      buildIndex([doc("home.md", "# Home\n"), doc("guides/deep/setup.md", "# Setup\n")]),
    );
    expect(tree.notes.map((n) => n.path)).toEqual(["home.md"]);
    const deep = tree.dirs.get("guides")!.dirs.get("deep")!;
    expect(deep.path).toBe("guides/deep");
    expect(deep.notes.map((n) => n.title)).toEqual(["Setup"]);
  });

  it("keeps a folder with no notes in it, empty or not (AGE-119, AGE-120)", () => {
    // "assets" holds only a PNG, so the scan reports it and no note does.
    const tree = buildDocsTree(buildIndex([doc("home.md", "# Home\n")], ["assets", "ideas"]));
    const ideas = tree.dirs.get("ideas")!;
    expect(ideas.path).toBe("ideas");
    expect(ideas.notes).toEqual([]);
    expect(tree.dirs.get("assets")!.notes).toEqual([]);
    expect(allDirPaths(tree).sort()).toEqual(["assets", "ideas"]);
  });

  it("fills in the ancestors of a nested note-less folder", () => {
    const tree = buildDocsTree(buildIndex([], ["research/2026/q3"]));
    expect(allDirPaths(tree).sort()).toEqual(["research", "research/2026", "research/2026/q3"]);
  });

  it("does not duplicate a folder that is both dir-listed and note-bearing", () => {
    // The scan reports every folder, so a note-bearing one is in both lists.
    const tree = buildDocsTree(buildIndex([doc("ideas/a.md", "# A\n")], ["ideas"]));
    expect([...tree.dirs.keys()]).toEqual(["ideas"]);
    expect(tree.dirs.get("ideas")!.notes.map((n) => n.title)).toEqual(["A"]);
  });

  it("sorts notes by title, newest-first in the journal", () => {
    const tree = buildDocsTree(
      buildIndex([
        doc("b.md", "# Beta\n"),
        doc("a.md", "# Alpha\n"),
        doc("journal/2026-01-01.md", "# Jan\n"),
        doc("journal/2026-02-01.md", "# Feb\n"),
      ]),
    );
    expect(tree.notes.map((n) => n.title)).toEqual(["Alpha", "Beta"]);
    expect(tree.dirs.get("journal")!.notes.map((n) => n.path)).toEqual([
      "journal/2026-02-01.md",
      "journal/2026-01-01.md",
    ]);
  });

  it("is an empty root when there is no index yet", () => {
    const tree = buildDocsTree(null);
    expect(tree.notes).toEqual([]);
    expect(allDirPaths(tree)).toEqual([]);
  });
});
