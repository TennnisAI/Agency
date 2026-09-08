import { describe, expect, it } from "vitest";
import { commitGuard, stagingAll, type CommitState } from "./commitGuard";
import type { FileChange } from "../../api";

const fc = (index: string, worktree: string, path = "f"): FileChange => ({ path, index, worktree });
const tree = (changes: FileChange[], merging = false): CommitState => ({ changes, merging });

describe("commitGuard", () => {
  it("lets a commit with staged changes through", () => {
    expect(commitGuard("commit", tree([fc("M", " ", "staged")]))).toEqual({ kind: "ok" });
  });

  it("offers to stage when nothing is staged but files have changed", () => {
    const changes = tree([fc(" ", "M", "edited"), fc("?", "?", "new")]);
    expect(commitGuard("commit", changes))
      .toEqual({ kind: "unstaged", files: 2, folders: 0, action: "commit" });
    expect(commitGuard("commitPush", changes))
      .toEqual({ kind: "unstaged", files: 2, folders: 0, action: "commitPush" });
  });

  it("counts a collapsed untracked folder as a folder, not a file", () => {
    // git reports an untracked folder over the expansion cap as one row; the
    // prompt calling that "1 file" is the only thing between the user and a
    // `git add -A` over every file inside it.
    const changes = tree([fc(" ", "M", "edited"), fc("?", "?", "vendor/")]);
    expect(commitGuard("commit", changes))
      .toEqual({ kind: "unstaged", files: 1, folders: 1, action: "commit" });
  });

  it("counts a file that is both staged and edited once, and lets it commit", () => {
    // The staged half is what the commit would take, so there is no offer.
    expect(commitGuard("commit", tree([fc("M", "M", "both")]))).toEqual({ kind: "ok" });
  });

  it("reports a clean tree as empty rather than offering to stage nothing", () => {
    expect(commitGuard("commit", tree([]))).toEqual({ kind: "empty" });
    expect(commitGuard("commitAll", tree([]))).toEqual({ kind: "empty" });
  });

  it("lets a clean tree commit while a merge is unconcluded", () => {
    // Every conflict resolved as "keep all current" leaves the index equal to
    // HEAD, so status is empty; the merge commit is still required (AGE-205).
    expect(commitGuard("commit", tree([], true))).toEqual({ kind: "ok" });
    expect(commitGuard("commitPush", tree([], true))).toEqual({ kind: "ok" });
  });

  it("refuses nothing when the status could not be read", () => {
    // A failed `git status` must not be reported as an empty working tree: git
    // gets to speak for itself, and its error is the one the banner shows.
    for (const action of ["commit", "commitAll", "commitPush", "commitAllPush", "amend"] as const) {
      expect(commitGuard(action, null)).toEqual({ kind: "ok" });
    }
  });

  it("lets the stage-everything variants run without a staged file", () => {
    const changes = tree([fc(" ", "M", "edited")]);
    expect(commitGuard("commitAll", changes)).toEqual({ kind: "ok" });
    expect(commitGuard("commitAllPush", changes)).toEqual({ kind: "ok" });
  });

  it("lets an amend through on an empty index: that is how a message is reworded", () => {
    expect(commitGuard("amend", tree([]))).toEqual({ kind: "ok" });
  });

  it("stops every action while a conflict is unresolved", () => {
    const changes = tree([fc("U", "U", "conflict"), fc("M", " ", "staged")], true);
    for (const action of ["commit", "commitAll", "commitPush", "commitAllPush", "amend"] as const) {
      expect(commitGuard(action, changes)).toEqual({ kind: "conflicts", count: 1 });
    }
  });
});

describe("stagingAll", () => {
  it("keeps the push half of Commit & Push", () => {
    expect(stagingAll("commitPush")).toBe("commitAllPush");
    expect(stagingAll("commit")).toBe("commitAll");
  });
});
