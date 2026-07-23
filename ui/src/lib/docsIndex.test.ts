import { describe, expect, it } from "vitest";
import { buildIndex, resolveLink, searchDocs } from "./docsIndex";
import { DocFile } from "../api";

const doc = (path: string, text: string): DocFile => ({ path, text, tooLarge: false });

describe("buildIndex", () => {
  it("extracts titles, headings, tags, and links", () => {
    const index = buildIndex([
      doc("home.md", "# Welcome\n\nSee [[Setup]] and [[guides/Advanced|the deep dive]].\n\n## Topics\n#intro #getting-started\n"),
      doc("guides/setup.md", "# Setup\n\nBack to [[Home]].\n"),
      doc("guides/advanced.md", "no h1 here\n"),
    ]);
    const home = index.docs.get("home.md")!;
    expect(home.title).toBe("Welcome");
    expect(home.headings.map((h) => [h.level, h.text])).toEqual([[1, "Welcome"], [2, "Topics"]]);
    expect(home.tags).toEqual(["intro", "getting-started"]);
    expect(home.links.map((l) => l.target)).toEqual(["Setup", "guides/Advanced"]);
    expect(home.links[1].alias).toBe("the deep dive");
    // A doc without an H1 falls back to its basename.
    expect(index.docs.get("guides/advanced.md")!.title).toBe("advanced");
  });

  it("builds backlinks from resolved links", () => {
    const index = buildIndex([
      doc("home.md", "See [[Setup]]\n"),
      doc("guides/setup.md", "# Setup\nBack to [[Home]]\n"),
    ]);
    expect(index.backlinks.get("guides/setup.md")).toEqual([
      { from: "home.md", line: 0, snippet: "See [[Setup]]" },
    ]);
    expect(index.backlinks.get("home.md")).toEqual([
      { from: "guides/setup.md", line: 1, snippet: "Back to [[Home]]" },
    ]);
  });

  it("ignores tags and links inside code", () => {
    const index = buildIndex([
      doc("a.md", "```\n[[NotALink]] #nottag\n```\n\n`#inline [[AlsoNot]]` but #real\n"),
    ]);
    const a = index.docs.get("a.md")!;
    expect(a.links).toEqual([]);
    expect(a.tags).toEqual(["real"]);
  });

  it("parses heading and alias parts of wikilinks", () => {
    const index = buildIndex([doc("a.md", "[[Note#Some Heading|shown]]\n")]);
    const link = index.docs.get("a.md")!.links[0];
    expect(link.target).toBe("Note");
    expect(link.heading).toBe("Some Heading");
    expect(link.alias).toBe("shown");
  });
});

describe("resolveLink", () => {
  const index = buildIndex([
    doc("note.md", "x"),
    doc("deep/nested/note.md", "x"),
    doc("guides/setup.md", "x"),
    doc("other.md", "x"),
  ]);

  it("resolves basenames case-insensitively", () => {
    expect(resolveLink(index, "Setup")).toBe("guides/setup.md");
    expect(resolveLink(index, "OTHER")).toBe("other.md");
  });

  it("prefers the shortest path on ambiguity", () => {
    expect(resolveLink(index, "note")).toBe("note.md");
  });

  it("resolves folder-qualified targets by path suffix", () => {
    expect(resolveLink(index, "nested/note")).toBe("deep/nested/note.md");
    expect(resolveLink(index, "guides/setup")).toBe("guides/setup.md");
  });

  it("returns null for unknown targets", () => {
    expect(resolveLink(index, "missing")).toBeNull();
    expect(resolveLink(index, "")).toBeNull();
  });
});

describe("searchDocs", () => {
  const index = buildIndex([
    doc("home.md", "# Welcome\n\nfind me here\n#alpha\n"),
    doc("b.md", "# Beta\n\nnothing\n#alphabet\n"),
  ]);

  it("matches text lines case-insensitively", () => {
    const hits = searchDocs(index, "FIND ME");
    expect(hits).toEqual([{ path: "home.md", line: 2, snippet: "find me here" }]);
  });

  it("matches titles", () => {
    expect(searchDocs(index, "welcome")[0]).toEqual({ path: "home.md", line: -1, snippet: "Welcome" });
  });

  it("matches tags by prefix with a # query", () => {
    const hits = searchDocs(index, "#alpha");
    expect(hits.map((h) => h.path).sort()).toEqual(["b.md", "home.md"]);
  });

  it("returns nothing for an empty query", () => {
    expect(searchDocs(index, "  ")).toEqual([]);
  });
});
