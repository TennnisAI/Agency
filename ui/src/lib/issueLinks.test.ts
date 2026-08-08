import { describe, expect, it } from "vitest";
import { CrossSource, LinkIssue, buildCrossRefs } from "./links";
import { issueLinks, linkCandidates, normalizeLinkKey, withLink, withoutLink } from "./issueLinks";

const issue = (over: Partial<LinkIssue> & Pick<LinkIssue, "id" | "projectId" | "seq">): LinkIssue => ({
  title: `Issue ${over.seq}`,
  body: "",
  status: "todo",
  links: [],
  ...over,
});

// AGE-2 links to AGE-14 and to an issue no project has; HOME-1 links back to
// AGE-2 from its own side only.
const sources: CrossSource[] = [
  {
    project: { id: "p1", name: "Agency", issue_key: "AGE" },
    issues: [
      issue({ id: "i14", projectId: "p1", seq: 14, title: "Fix terminal resize" }),
      issue({ id: "i2", projectId: "p1", seq: 2, links: ["AGE-14", "ZZZ-9"] }),
    ],
    runs: [],
  },
  {
    project: { id: "ws", name: "Workspace", issue_key: "HOME" },
    issues: [issue({ id: "h1", projectId: "ws", seq: 1, links: ["AGE-2"] })],
    runs: [],
  },
];
const cross = buildCrossRefs(sources);
const age2 = sources[0].issues[1];

describe("withLink / withoutLink", () => {
  it("adds normalized, never twice, and removes case-insensitively", () => {
    expect(withLink([], "age-3")).toEqual(["AGE-3"]);
    expect(withLink(["AGE-3"], "age-3")).toEqual(["AGE-3"]);
    expect(withLink(["AGE-3"], "AGE-4")).toEqual(["AGE-3", "AGE-4"]);
    expect(withoutLink(["AGE-3", "AGE-4"], "age-3")).toEqual(["AGE-4"]);
    expect(normalizeLinkKey("  age-3 ")).toBe("AGE-3");
  });
});

describe("issueLinks", () => {
  it("resolves own links, keeps unresolvable keys, and adds the other side's", () => {
    const rows = issueLinks(age2, "AGE-2", cross);
    expect(rows.map((r) => [r.label, r.ref?.issue.id ?? null, r.outgoing])).toEqual([
      ["AGE-14", "i14", true],
      // No project has a ZZZ key: the row stays, unresolved.
      ["ZZZ-9", null, true],
      // HOME-1 says it links to AGE-2, though AGE-2 doesn't say so back.
      ["HOME-1", "h1", false],
    ]);
  });

  it("drops self-links and duplicates, and works before cross refs load", () => {
    const self = issue({ id: "i2", projectId: "p1", seq: 2, links: ["AGE-2", "AGE-14", "age-14"] });
    expect(issueLinks(self, "AGE-2", cross).map((r) => r.label)).toEqual(["AGE-14", "HOME-1"]);
    // Nothing loaded: own links still render, unresolved, with no inbound half.
    expect(issueLinks(age2, "AGE-2", null).map((r) => [r.label, r.ref])).toEqual([
      ["AGE-14", null],
      ["ZZZ-9", null],
    ]);
  });
});

describe("linkCandidates", () => {
  it("offers every other issue, own project first, minus the linked ones", () => {
    const rows = issueLinks(age2, "AGE-2", cross);
    expect(linkCandidates(cross, "p1", "AGE-2", rows).map((r) => r.issue.id)).toEqual([]);
    // From AGE-14's side nothing is linked yet, so both others are on offer,
    // this project's first.
    expect(linkCandidates(cross, "p1", "AGE-14", []).map((r) => r.issue.id)).toEqual(["i2", "h1"]);
  });
});
