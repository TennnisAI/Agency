import { describe, expect, it } from "vitest";
import { RepoReadiness } from "../api";
import { gitlessAnswer, isGitless, pathsToProbe } from "./useRepoReadiness";

const readiness = (state: RepoReadiness["state"]): RepoReadiness =>
  ({ state, stageable: false, dirty: false });

describe("isGitless", () => {
  it("is true only for a folder with no repository", () => {
    expect(isGitless(readiness("notARepo"))).toBe(true);
    expect(isGitless(readiness("noCommits"))).toBe(false);
    expect(isGitless(readiness("ready"))).toBe(false);
  });
  it("claims nothing while the first inspection is in flight", () => {
    expect(isGitless(null)).toBe(false);
  });
  // AGE-203: a folder that is gone is not a folder with no repository in it.
  // Everything gated on `gitless` reads it as "no branch here, so let the agent
  // work in the folder itself", and there is no folder to work in.
  it("is false for a folder that has gone from disk", () => {
    expect(isGitless(readiness("missing"))).toBe(false);
  });
});

describe("gitlessAnswer", () => {
  it("answers for a folder that is there", () => {
    expect(gitlessAnswer(readiness("notARepo"))).toBe(true);
    expect(gitlessAnswer(readiness("ready"))).toBe(false);
  });
  // A `false` is latched for the life of the app by `pathsToProbe`, so a
  // gitless project on a disk that happened to be unplugged during the first
  // sweep would keep its branch-shaped entries forever after the disk came back.
  it("records nothing for a folder that has gone from disk", () => {
    expect(gitlessAnswer(readiness("missing"))).toBeNull();
  });
});

describe("pathsToProbe", () => {
  it("probes every path when nothing is known yet", () => {
    expect(pathsToProbe(["/a", "/b"], {})).toEqual(["/a", "/b"]);
  });
  it("leaves known repositories alone", () => {
    expect(pathsToProbe(["/a", "/b"], { "/a": false })).toEqual(["/b"]);
  });
  it("re-probes a folder last seen gitless, so a git init elsewhere lands", () => {
    expect(pathsToProbe(["/a"], { "/a": true })).toEqual(["/a"]);
  });
  it("asks once for a folder two projects share", () => {
    expect(pathsToProbe(["/a", "/a"], {})).toEqual(["/a"]);
  });
  it("ignores answers for paths no longer listed", () => {
    expect(pathsToProbe([], { "/gone": true })).toEqual([]);
  });
});
