import { describe, expect, it } from "vitest";
import {
  SearchHit,
  buildIndex,
  fmFilterPaths,
  isValidFmKey,
  mergeBodyHits,
  parseFrontmatter,
  resolveLink,
  searchDocs,
  searchLocal,
  serializeFrontmatter,
} from "./docsIndex";
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

describe("frontmatter", () => {
  it("parses ordered pairs and reports the body start", () => {
    const meta = buildIndex([
      doc("a.md", '---\nstatus: draft\ntype: "plan"\nstatus: extra\n---\n# Real Title\n#tag\n'),
    ]).docs.get("a.md")!;
    expect(meta.frontmatter).toEqual([["status", "draft"], ["type", "plan"], ["status", "extra"]]);
    expect(meta.fmEnd).toBe(5);
    expect(meta.title).toBe("Real Title");
    expect(meta.tags).toEqual(["tag"]);
  });

  it("keeps empty values and blank lines", () => {
    const fm = parseFrontmatter(["---", "due:", "", "owner: nic", "---", "body"]);
    expect(fm).toEqual({ pairs: [["due", ""], ["owner", "nic"]], end: 5 });
  });

  it("rejects malformed frontmatter as body", () => {
    // No fence on line 1.
    expect(parseFrontmatter(["x", "---", "k: v", "---"])).toBeNull();
    // Unclosed fence.
    expect(parseFrontmatter(["---", "k: v"])).toBeNull();
    // A line that isn't key: value.
    expect(parseFrontmatter(["---", "just prose", "---"])).toBeNull();
    const meta = buildIndex([doc("a.md", "---\nnot a pair\n---\n")]).docs.get("a.md")!;
    expect(meta.frontmatter).toEqual([]);
    expect(meta.fmEnd).toBe(0);
  });

  it("does not collect headings, tags, or links from frontmatter", () => {
    const meta = buildIndex([
      doc("a.md", "---\nref: [[Setup]]\nnote: #notatag\n---\nbody\n"),
      doc("setup.md", "# Setup\n"),
    ]).docs.get("a.md")!;
    expect(meta.links).toEqual([]);
    expect(meta.tags).toEqual([]);
    expect(meta.headings).toEqual([]);
    expect(meta.title).toBe("a");
  });

  it("treats a mid-document --- as body, not frontmatter", () => {
    const meta = buildIndex([doc("a.md", "# T\n\n---\n\nmore\n")]).docs.get("a.md")!;
    expect(meta.frontmatter).toEqual([]);
    expect(meta.fmEnd).toBe(0);
  });

  it("serializeFrontmatter round-trips through parseFrontmatter", () => {
    const pairs: [string, string][] = [["status", "draft"], ["due", ""], ["status", "extra"]];
    const text = serializeFrontmatter(pairs);
    expect(text).toBe("---\nstatus: draft\ndue:\nstatus: extra\n---\n");
    expect(parseFrontmatter(text.split("\n"))).toEqual({ pairs, end: 5 });
    expect(serializeFrontmatter([])).toBe("");
  });

  it("isValidFmKey matches what the parser accepts", () => {
    expect(isValidFmKey("status")).toBe(true);
    expect(isValidFmKey("my-key_2")).toBe(true);
    expect(isValidFmKey("2fast")).toBe(false);
    expect(isValidFmKey("has space")).toBe(false);
    expect(isValidFmKey("")).toBe(false);
  });

  it("an empty frontmatter block parses as zero pairs", () => {
    expect(parseFrontmatter(["---", "---", "body"])).toEqual({ pairs: [], end: 2 });
  });
});

describe("key:value search", () => {
  const index = buildIndex([
    doc("plan.md", "---\nstatus: draft\ntype: plan\n---\n# Budget Plan\n\nneedle body\n"),
    doc("done.md", "---\nstatus: shipped\n---\n# Done Thing\n"),
    doc("plain.md", "# Plain\n\nneedle body\n"),
  ]);

  it("filter-only queries return matching docs as title hits", () => {
    expect(searchLocal(index, "status:draft")).toEqual([
      { path: "plan.md", line: -1, snippet: "Budget Plan" },
    ]);
  });

  it("matches values by case-insensitive substring and bare key: by presence", () => {
    expect(searchLocal(index, "status:SHIP").map((h) => h.path)).toEqual(["done.md"]);
    expect(searchLocal(index, "status:").map((h) => h.path).sort()).toEqual(["done.md", "plan.md"]);
    expect(searchLocal(index, "missing:x")).toEqual([]);
  });

  it("combines filters with free text over titles", () => {
    expect(searchLocal(index, "status:draft budget").map((h) => h.path)).toEqual(["plan.md"]);
    expect(searchLocal(index, "status:draft done")).toEqual([]);
  });

  it("fmFilterPaths returns the allowed set and mergeBodyHits honors it", () => {
    const allowed = fmFilterPaths(index, [["status", "draft"]]);
    expect([...allowed]).toEqual(["plan.md"]);
    const merged = mergeBodyHits(
      index,
      [],
      [
        { path: "plan.md", line: 7, col: 1, text: "needle body" },
        { path: "plain.md", line: 3, col: 1, text: "needle body" },
      ],
      allowed,
    );
    expect(merged.map((h) => h.path)).toEqual(["plan.md"]);
  });

  it("searchDocs fallback applies filters too", () => {
    expect(searchDocs(index, "status:draft needle")).toEqual([
      { path: "plan.md", line: 6, snippet: "needle body" },
    ]);
    expect(searchDocs(index, "type:plan").map((h) => h.path)).toEqual(["plan.md"]);
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

describe("searchLocal + mergeBodyHits", () => {
  const index = buildIndex([
    doc("alpha.md", "# Alpha Guide\n\nbody mentions needle\nagain needle\n#projx\n"),
    doc("beta.md", "# Beta\n\nneedle here too\n"),
  ]);

  it("answers tag queries entirely from the index", () => {
    const hits = searchLocal(index, "#proj");
    expect(hits).toEqual([{ path: "alpha.md", line: -1, snippet: "#projx" }]);
  });

  it("returns title hits only (bodies belong to the backend)", () => {
    const hits = searchLocal(index, "alpha");
    expect(hits).toEqual([{ path: "alpha.md", line: -1, snippet: "Alpha Guide" }]);
    expect(searchLocal(index, "needle")).toEqual([]);
  });

  it("maps backend hits to 0-based lines with trimmed snippets", () => {
    const merged = mergeBodyHits(index, [], [
      { path: "beta.md", line: 3, col: 1, text: "needle here too\n" },
    ]);
    expect(merged).toEqual([{ path: "beta.md", line: 2, snippet: "needle here too" }]);
  });

  it("keeps title hits first, drops unknown paths, and caps per doc", () => {
    const local = searchLocal(index, "alpha");
    const body = [
      { path: "alpha.md", line: 3, col: 15, text: "body mentions needle" },
      { path: "not-in-corpus.txt", line: 1, col: 1, text: "needle" },
      ...Array.from({ length: 10 }, (_, i) => ({
        path: "alpha.md", line: 10 + i, col: 1, text: `filler ${i}`,
      })),
    ];
    const merged = mergeBodyHits(index, local, body);
    expect(merged[0]).toEqual({ path: "alpha.md", line: -1, snippet: "Alpha Guide" });
    expect(merged.every((h: SearchHit) => h.path !== "not-in-corpus.txt")).toBe(true);
    expect(merged.filter((h: SearchHit) => h.path === "alpha.md").length).toBe(5);
  });
});
