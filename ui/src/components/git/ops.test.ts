import { describe, it, expect, beforeEach } from "vitest";
import { gitOp, setGitOp, clearGitOps, isCancelled } from "./ops";

describe("git op store", () => {
  beforeEach(clearGitOps);

  it("starts idle for an untouched repo", () => {
    expect(gitOp("project:a")).toEqual({ busy: false, progress: null, error: "" });
  });

  it("keeps each repo's op to itself", () => {
    setGitOp("project:a", { busy: true, progress: { phase: "Pushing…", percent: 40, detail: "" } });
    expect(gitOp("project:a").busy).toBe(true);
    expect(gitOp("project:b")).toEqual({ busy: false, progress: null, error: "" });
  });

  it("merges patches into the existing op", () => {
    setGitOp("project:a", { busy: true });
    setGitOp("project:a", { progress: { phase: "Writing objects", percent: 10, detail: "" } });
    expect(gitOp("project:a").busy).toBe(true);
    expect(gitOp("project:a").progress?.percent).toBe(10);
  });

  it("returns a stable snapshot while nothing changes", () => {
    setGitOp("project:a", { busy: true });
    expect(gitOp("project:a")).toBe(gitOp("project:a"));
    expect(gitOp("project:b")).toBe(gitOp("project:c")); // idle repos share one object
  });

  it("goes back to idle once the op settles", () => {
    setGitOp("project:a", { busy: true, progress: { phase: "Pushing…", percent: null, detail: "" } });
    setGitOp("project:a", { busy: false });
    setGitOp("project:a", { progress: null });
    expect(gitOp("project:a")).toEqual({ busy: false, progress: null, error: "" });
  });

  it("reads a cancelled push as the user's own doing, not a failure", () => {
    setGitOp("project:a", { busy: true });
    setGitOp("project:a", { error: "cancelled" });
    setGitOp("project:a", { busy: false });
    expect(isCancelled(gitOp("project:a").error)).toBe(true);
  });

  it("still reports a push that git itself says was cancelled", () => {
    // A server-side hook's wording must not be mistaken for the Cancel button:
    // swallowing it would leave the panel looking as if nothing had happened.
    expect(isCancelled("remote: push cancelled by policy")).toBe(false);
    expect(isCancelled("")).toBe(false);
  });

  it("holds a failed op's error after it stops being busy", () => {
    setGitOp("project:a", { busy: true, error: "" });
    setGitOp("project:a", { error: "! [rejected] main -> main" });
    setGitOp("project:a", { busy: false });
    expect(gitOp("project:a").error).toBe("! [rejected] main -> main");
    expect(gitOp("project:b").error).toBe("");
  });
});
