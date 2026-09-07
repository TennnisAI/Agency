import { describe, expect, it } from "vitest";
import { commitGuard, stagingAll } from "./commitGuard";
import type { FileChange } from "../../api";

const fc = (index: string, worktree: string, path = "f"): FileChange => ({ path, index, worktree });

describe("commitGuard", () => {
  it("lets a commit with staged changes through", () => {
    expect(commitGuard("commit", [fc("M", " ", "staged")])).toEqual({ kind: "ok" });
  });

  it("offers to stage when nothing is staged but files have changed", () => {
    const changes = [fc(" ", "M", "edited"), fc("?", "?", "new")];
    expect(commitGuard("commit", changes)).toEqual({ kind: "unstaged", count: 2 });
    expect(commitGuard("commitPush", changes)).toEqual({ kind: "unstaged", count: 2 });
  });

  it("counts a file that is both staged and edited once, and lets it commit", () => {
    // The staged half is what the commit would take, so there is no offer.
    expect(commitGuard("commit", [fc("M", "M", "both")])).toEqual({ kind: "ok" });
  });

  it("reports a clean tree as empty rather than offering to stage nothing", () => {
    expect(commitGuard("commit", [])).toEqual({ kind: "empty" });
    expect(commitGuard("commitAll", [])).toEqual({ kind: "empty" });
  });

  it("lets the stage-everything variants run without a staged file", () => {
    const changes = [fc(" ", "M", "edited")];
    expect(commitGuard("commitAll", changes)).toEqual({ kind: "ok" });
    expect(commitGuard("commitAllPush", changes)).toEqual({ kind: "ok" });
  });

  it("lets an amend through on an empty index: that is how a message is reworded", () => {
    expect(commitGuard("amend", [])).toEqual({ kind: "ok" });
  });

  it("stops every action while a conflict is unresolved", () => {
    const changes = [fc("U", "U", "conflict"), fc("M", " ", "staged")];
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
