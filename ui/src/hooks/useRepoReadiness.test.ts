import { describe, expect, it } from "vitest";
import { RepoReadiness } from "../api";
import { isGitless, pathsToProbe } from "./useRepoReadiness";

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
