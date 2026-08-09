import { describe, expect, it } from "vitest";
import { DocFile } from "../api";
import { buildIndex } from "../lib/docsIndex";
import { CrossSource, LinkIssue, buildCrossRefs } from "../lib/links";
import { crossRefsSignature, docsIndexSignature } from "./useLiveFacets";

const doc = (path: string, text: string): DocFile => ({ path, text, tooLarge: false });

const issue = (over: Partial<LinkIssue> & Pick<LinkIssue, "id" | "projectId" | "seq">): LinkIssue => ({
  title: `Issue ${over.seq}`,
  body: "",
  status: "todo",
  links: [],
  ...over,
});

const sources = (over: Partial<CrossSource> = {}): CrossSource[] => [{
  project: { id: "p1", name: "Agency", issue_key: "AGE" },
  issues: [issue({ id: "i14", projectId: "p1", seq: 14, title: "Fix terminal resize" })],
  runs: [{ id: "r1", projectId: "p1", issueId: "i14" }],
  ...over,
}];

describe("docsIndexSignature", () => {
  const files = [
    doc("Plan.md", "---\ntype: plan\n---\n# The Plan\n\nSee [[Setup]]. #roadmap\n"),
    doc("guides/Setup.md", "# Setup\n"),
  ];

  it("is stable across a rebuild of the same corpus", () => {
    expect(docsIndexSignature(buildIndex(files))).toBe(docsIndexSignature(buildIndex(files)));
  });

  it("ignores body edits the editor never reads", () => {
    const edited = [doc("Plan.md", "---\ntype: plan\n---\n# The Plan\n\nSee [[Setup]]. #roadmap\n\nA new paragraph.\n"), files[1]];
    expect(docsIndexSignature(buildIndex(edited))).toBe(docsIndexSignature(buildIndex(files)));
  });

  it("changes when a note is added, renamed, or retitled", () => {
    const base = docsIndexSignature(buildIndex(files));
    expect(docsIndexSignature(buildIndex([...files, doc("Notes.md", "# Notes\n")]))).not.toBe(base);
    expect(docsIndexSignature(buildIndex([doc("Roadmap.md", files[0].text), files[1]]))).not.toBe(base);
    expect(docsIndexSignature(buildIndex([doc("Plan.md", "# Renamed\n"), files[1]]))).not.toBe(base);
  });

  it("changes when a tag or a frontmatter pair changes", () => {
    const base = docsIndexSignature(buildIndex(files));
    const retagged = [doc("Plan.md", files[0].text.replace("#roadmap", "#later")), files[1]];
    expect(docsIndexSignature(buildIndex(retagged))).not.toBe(base);
    const refm = [doc("Plan.md", files[0].text.replace("type: plan", "type: spec")), files[1]];
    expect(docsIndexSignature(buildIndex(refm))).not.toBe(base);
  });

  it("separates a loading index from an empty corpus", () => {
    expect(docsIndexSignature(null)).toBe("");
    expect(docsIndexSignature(buildIndex([]))).not.toBe(docsIndexSignature(null));
  });
});

describe("crossRefsSignature", () => {
  it("is stable across a rebuild of the same rows", () => {
    expect(crossRefsSignature(buildCrossRefs(sources())))
      .toBe(crossRefsSignature(buildCrossRefs(sources())));
  });

  it("ignores issue fields the editor never reads", () => {
    const edited = sources({
      issues: [issue({
        id: "i14", projectId: "p1", seq: 14, title: "Fix terminal resize",
        body: "A rewritten body with a [[Plan]] link.", status: "done", links: ["AGE-2"],
      })],
    });
    expect(crossRefsSignature(buildCrossRefs(edited)))
      .toBe(crossRefsSignature(buildCrossRefs(sources())));
  });

  it("changes when a label, title, run, or project key changes", () => {
    const base = crossRefsSignature(buildCrossRefs(sources()));
    const reseq = sources({ issues: [issue({ id: "i14", projectId: "p1", seq: 15, title: "Fix terminal resize" })] });
    expect(crossRefsSignature(buildCrossRefs(reseq))).not.toBe(base);
    const retitled = sources({ issues: [issue({ id: "i14", projectId: "p1", seq: 14, title: "Fix pane resize" })] });
    expect(crossRefsSignature(buildCrossRefs(retitled))).not.toBe(base);
    const newRun = sources({ runs: [{ id: "r1", projectId: "p1", issueId: "i14" }, { id: "r2", projectId: "p1", issueId: null }] });
    expect(crossRefsSignature(buildCrossRefs(newRun))).not.toBe(base);
    const rekeyed = sources({ project: { id: "p1", name: "Agency", issue_key: "AGY" } });
    expect(crossRefsSignature(buildCrossRefs(rekeyed))).not.toBe(base);
  });

  it("separates loading cross refs from a workspace with no issues", () => {
    expect(crossRefsSignature(null)).toBe("");
    expect(crossRefsSignature(buildCrossRefs([]))).not.toBe(crossRefsSignature(null));
  });
});
