import { describe, expect, it } from "vitest";
import { needsGitCredentials, promptedUsername } from "./gitCredentials";

const EXPIRED =
  "git push failed:\nfatal: could not read Password for 'https://nic@github.com': terminal prompts disabled";

describe("needsGitCredentials", () => {
  it("recognises an expired token on push and fetch", () => {
    expect(needsGitCredentials(EXPIRED)).toBe(true);
    expect(
      needsGitCredentials(
        new Error(`git ["fetch", "--prune", "origin"] failed: fatal: could not read Username for 'https://github.com': terminal prompts disabled`),
      ),
    ).toBe(true);
    expect(needsGitCredentials("fatal: Authentication failed for 'https://github.com/a/b.git/'")).toBe(true);
  });

  it("leaves other push failures to the banner", () => {
    expect(needsGitCredentials("! [rejected] main -> main (fetch first)")).toBe(false);
    expect(needsGitCredentials("Could not resolve host: github.com")).toBe(false);
    expect(needsGitCredentials(null)).toBe(false);
  });
});

describe("promptedUsername", () => {
  it("reads the username git asked the password for", () => {
    expect(promptedUsername(EXPIRED)).toBe("nic");
    expect(promptedUsername("could not read Username for 'https://github.com'")).toBe(null);
  });
});
