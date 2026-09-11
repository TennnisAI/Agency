import { describe, expect, it } from "vitest";
import { needsGitIdentity } from "./gitIdentity";

describe("needsGitIdentity", () => {
  it("recognises the fresh-install commit failure", () => {
    expect(
      needsGitIdentity(
        new Error(
          'git ["commit", "-m", "Initial commit"] failed: Author identity unknown\n\n*** Please tell me who you are.',
        ),
      ),
    ).toBe(true);
    expect(needsGitIdentity("empty ident name (for <nic@x>) not allowed")).toBe(true);
  });

  it("leaves unrelated git errors to the normal toast", () => {
    expect(needsGitIdentity("git add -A failed: permission denied")).toBe(false);
    expect(needsGitIdentity("nothing to commit, working tree clean")).toBe(false);
    expect(needsGitIdentity(null)).toBe(false);
  });
});
