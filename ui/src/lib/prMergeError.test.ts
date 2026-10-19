import { describe, expect, it } from "vitest";
import { prMergeError } from "./prMergeError";

// Errors as the backend hands them over: `run_ok` quotes gh's arguments, the
// pin included, ahead of gh's own stderr.
const failed = (stderr: string) =>
  `gh ["pr", "merge", "3", "--squash", "--match-head-commit", "abc123"] failed: ${stderr}`;

describe("prMergeError", () => {
  it("reads a real conflict as one", () => {
    const e = failed("X Pull request owner/repo#3 is not mergeable: the merge commit cannot be cleanly created.");
    expect(prMergeError(e, "main")).toMatch(/conflicts with main/);
  });

  it("keeps gh's words for a protected branch or a stale head", () => {
    for (const reason of [
      "the base branch policy prohibits the merge",
      "the head branch is not up to date with the base branch",
    ]) {
      const e = failed(`X Pull request owner/repo#3 is not mergeable: ${reason}.`);
      expect(prMergeError(e, "main")).toBe(e);
    }
  });

  it("reads a tripped pin as a mismatch", () => {
    const e = failed("GraphQL: Head branch was modified. Review and try the merge again. (mergePullRequest)");
    expect(prMergeError(e, "main")).toMatch(/doesn't match this agent's branch/);
  });

  it("does not mistake the pin's flag in the arguments for a mismatch", () => {
    const e = failed("GraphQL: Pull request is in draft state (mergePullRequest)");
    expect(prMergeError(e, "main")).toBe(e);
  });
});
