import { describe, expect, it } from "vitest";
import { missingToolFor } from "./missingTool";

describe("missingToolFor", () => {
  it("recognises the backend's missing-git sentences", () => {
    expect(missingToolFor("git is not installed")).toBe("git");
    expect(
      missingToolFor(new Error("git is not installed, so Agency cannot tell whether /x is a repository")),
    ).toBe("git");
  });

  it("does not fire on the unspecific wording, which may be a missing folder", () => {
    expect(
      missingToolFor("could not run git in /x to tell whether it is a repository; check that git is installed and the folder is available"),
    ).toBeNull();
    expect(missingToolFor("No such file or directory (os error 2)")).toBeNull();
    expect(missingToolFor(null)).toBeNull();
  });

  it("recognises a shell that could not find npm", () => {
    expect(missingToolFor("sh: 1: npm: not found")).toBe("node");
    expect(missingToolFor("bash: npm: command not found")).toBe("node");
  });
});
