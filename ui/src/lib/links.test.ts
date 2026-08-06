import { describe, expect, it } from "vitest";
import { DocFile } from "../api";
import { buildIndex, extractWikilinks } from "./docsIndex";
import {
  CrossSource, LinkIssue, LinkRun,
  buildCrossRefs, buildLinkIndex, classifyTarget, issueCompletionOptions,
  linkKey, mentionsOf, noteId, resolveTarget, wikilinkView,
} from "./links";

const doc = (path: string, text: string): DocFile => ({ path, text, tooLarge: false });

const issue = (over: Partial<LinkIssue> & Pick<LinkIssue, "id" | "projectId" | "seq">): LinkIssue => ({
  title: `Issue ${over.seq}`,
  body: "",
  status: "todo",
  links: [],
  ...over,
});

const run = (id: string, projectId: string, issueId: string | null = null): LinkRun => ({
  id, projectId, issueId,
});

// Two projects (a repo + the workspace) with issues and one run.
const sources: CrossSource[] = [
  {
    project: { id: "p1", name: "Agency", issue_key: "AGE" },
    issues: [
      issue({ id: "i14", projectId: "p1", seq: 14, title: "Fix terminal resize" }),
      issue({ id: "i2", projectId: "p1", seq: 2, body: "Blocked on [[AGE-14]] and [[Plan]]." }),
    ],
    runs: [run("r1", "p1", "i14")],
  },
  {
    project: { id: "ws", name: "Workspace", issue_key: "HOME" },
    issues: [issue({ id: "h1", projectId: "ws", seq: 1 })],
    runs: [],
  },
];
const cross = buildCrossRefs(sources);

describe("extractWikilinks", () => {
  it("finds targets with heading and alias parts, 0-based lines", () => {
    const links = extractWikilinks("first\nsee [[Note#Head|shown]] and [[Other]]\n");
    expect(links.map((l) => [l.target, l.heading, l.alias, l.line])).toEqual([
      ["Note", "Head", "shown", 1],
      ["Other", null, null, 1],
    ]);
  });

  it("skips fenced blocks and inline code", () => {
    const links = extractWikilinks("```\n[[NotALink]]\n```\n`[[AlsoNot]]` but [[Real]]\n");
    expect(links.map((l) => l.target)).toEqual(["Real"]);
  });
});

describe("classifyTarget", () => {
  const prefixes = new Set(["age", "3dp"]);

  it("classifies run:, known KEY-n, and note targets", () => {
    expect(classifyTarget("run:abc-123")).toEqual({ kind: "run", id: "abc-123" });
    expect(classifyTarget("AGE-14", prefixes)).toEqual({ kind: "issuePattern", prefix: "age", label: "age-14" });
    expect(classifyTarget("age-14", prefixes)).toEqual({ kind: "issuePattern", prefix: "age", label: "age-14" });
    // Keys can lead with a digit (derive_issue_key: "3D Print" → 3DP).
    expect(classifyTarget("3dp-7", prefixes)).toEqual({ kind: "issuePattern", prefix: "3dp", label: "3dp-7" });
    expect(classifyTarget("Note", prefixes)).toEqual({ kind: "note" });
    expect(classifyTarget("guides/Setup", prefixes)).toEqual({ kind: "note" });
  });

  it("keeps unknown prefixes and date-like names in the note domain", () => {
    // Key-shaped but no project uses FOO: a note target.
    expect(classifyTarget("FOO-3", prefixes)).toEqual({ kind: "note" });
    // While cross refs load there are no known keys — everything is a note.
    expect(classifyTarget("AGE-14")).toEqual({ kind: "note" });
    // Daily/monthly note names fit the shape but never a known key.
    expect(classifyTarget("2026-07-31", prefixes)).toEqual({ kind: "note" });
    expect(classifyTarget("2026-07", prefixes)).toEqual({ kind: "note" });
  });
});

describe("buildCrossRefs", () => {
  it("keys issues by case-insensitive label across projects", () => {
    expect(cross.issuesByLabel.get("age-14")?.issue.id).toBe("i14");
    expect(cross.issuesByLabel.get("home-1")?.issue.id).toBe("h1");
    expect(cross.keyPrefixes).toEqual(new Set(["age", "home"]));
    expect(cross.runsById.get("r1")?.project.id).toBe("p1");
  });
});

describe("resolveTarget", () => {
  const index = buildIndex([doc("plan.md", "# Plan\n"), doc("foo-3.md", "# Foo three\n")]);

  it("resolves issues across projects, case-insensitively", () => {
    const res = resolveTarget(index, cross, "age-14");
    expect(res).toMatchObject({ kind: "issue", ref: { issue: { id: "i14" } } });
  });

  it("known prefix with a missing seq is unresolved, never a note fallback", () => {
    expect(resolveTarget(index, cross, "AGE-999")).toEqual({ kind: "unresolvedIssue", label: "AGE-999" });
  });

  it("unknown prefix falls through to the note domain", () => {
    expect(resolveTarget(index, cross, "FOO-3")).toEqual({ kind: "note", path: "foo-3.md" });
    expect(resolveTarget(index, cross, "FOO-4")).toEqual({ kind: "unresolvedNote" });
  });

  it("resolves digit-leading issue keys end to end", () => {
    const threeD: CrossSource[] = [{
      project: { id: "p3", name: "3D Print", issue_key: "3DP" },
      issues: [issue({ id: "d1", projectId: "p3", seq: 1 })],
      runs: [],
    }];
    const c3 = buildCrossRefs(threeD);
    expect(resolveTarget(index, c3, "3dp-1")).toMatchObject({ kind: "issue", ref: { issue: { id: "d1" } } });
    expect(resolveTarget(index, c3, "3DP-9")).toEqual({ kind: "unresolvedIssue", label: "3DP-9" });
  });

  it("resolves runs and reports missing ones", () => {
    expect(resolveTarget(index, cross, "run:r1")).toMatchObject({ kind: "run", ref: { run: { id: "r1" } } });
    expect(resolveTarget(index, cross, "run:gone")).toEqual({ kind: "unresolvedRun", id: "gone" });
  });

  it("degrades to note behavior when cross refs haven't loaded", () => {
    expect(resolveTarget(index, null, "AGE-14")).toEqual({ kind: "unresolvedNote" });
    expect(resolveTarget(index, null, "Plan")).toEqual({ kind: "note", path: "plan.md" });
  });
});

describe("wikilinkView", () => {
  const index = buildIndex([doc("plan.md", "# Plan\n")]);

  it("flags unresolved issues and notes, with per-kind tooltips", () => {
    expect(wikilinkView(index, cross, "AGE-14")).toEqual({ unresolved: false, title: "⌘-click to open AGE-14" });
    expect(wikilinkView(index, cross, "AGE-999")).toEqual({ unresolved: true, title: "No matching issue" });
    expect(wikilinkView(index, cross, "Missing").unresolved).toBe(true);
    expect(wikilinkView(index, cross, "run:gone")).toEqual({ unresolved: true, title: "No matching run" });
  });

  it("never flags while reference data is loading", () => {
    expect(wikilinkView(null, null, "Missing").unresolved).toBe(false);
    expect(wikilinkView(index, null, "run:gone").unresolved).toBe(false);
  });
});

describe("buildLinkIndex", () => {
  const wsIndex = buildIndex([
    doc("journal/2026-07-31.md", "# Today\n\nShipped [[AGE-14]] via [[run:r1]].\n"),
  ]);
  const p1Index = buildIndex([doc("plan.md", "# Plan\n\nSee [[AGE-14]].\n")]);
  const corpora = [
    { project: sources[1].project, index: wsIndex },
    { project: sources[0].project, index: p1Index },
  ];
  const table = buildLinkIndex(corpora, cross);

  it("collects note→issue edges with title, snippet, and line", () => {
    const mentions = mentionsOf(table, "issue", "i14");
    const fromNotes = mentions.filter((m) => m.fromKind === "note");
    expect(fromNotes.map((m) => [m.fromProjectId, m.fromId, m.fromTitle, m.line])).toEqual([
      ["ws", "journal/2026-07-31.md", "Today", 2],
      ["p1", "plan.md", "Plan", 2],
    ]);
    expect(fromNotes[0].snippet).toBe("Shipped [[AGE-14]] via [[run:r1]].");
  });

  it("collects note→run edges", () => {
    expect(mentionsOf(table, "run", "r1").map((m) => m.fromId)).toEqual(["journal/2026-07-31.md"]);
  });

  it("collects issue→issue and issue→note edges from bodies", () => {
    const issueMentions = mentionsOf(table, "issue", "i14").filter((m) => m.fromKind === "issue");
    expect(issueMentions.map((m) => [m.fromId, m.fromLabel])).toEqual([["i2", "AGE-2"]]);
    // [[Plan]] in AGE-2's body resolves against its own project corpus.
    expect(mentionsOf(table, "note", noteId("p1", "plan.md")).map((m) => m.fromId)).toEqual(["i2"]);
  });

  it("drops self-links and leaves note→note to the docs backlinks", () => {
    const selfIndex = buildIndex([doc("a.md", "[[b]]\n"), doc("b.md", "x\n")]);
    const selfSources: CrossSource[] = [{
      project: { id: "p", name: "P", issue_key: "P" },
      issues: [
        issue({ id: "s1", projectId: "p", seq: 1, body: "self [[P-1]]" }),
        issue({ id: "s2", projectId: "p", seq: 2, body: "see [[P-1]]" }),
      ],
      runs: [],
    }];
    const t = buildLinkIndex([{ project: selfSources[0].project, index: selfIndex }], buildCrossRefs(selfSources));
    // s2's link lands (single-letter keys classify), s1's self-link is dropped.
    expect(mentionsOf(t, "issue", "s1").map((m) => m.fromId)).toEqual(["s2"]);
    expect(t.get(linkKey("note", noteId("p", "b.md")))).toBeUndefined();
  });
});

describe("issueCompletionOptions", () => {
  it("labels by project key, sorted by project then seq", () => {
    expect(issueCompletionOptions(cross).map((o) => o.label)).toEqual(["AGE-2", "AGE-14", "HOME-1"]);
    expect(issueCompletionOptions(cross)[1].detail).toBe("Fix terminal resize");
  });
});
