import { describe, it, expect } from "vitest";
import { repoSetupView } from "./repoSetupView";

describe("repoSetupView", () => {
  it("notARepo → init prompt, no gitignore toggle", () => {
    const v = repoSetupView({ state: "notARepo", stageable: false, dirty: false }, "add");
    expect(v.kind).toBe("init");
    expect(v.title).toMatch(/Set up/i);
    expect(v.showGitignore).toBe(false);
    expect(v.secondaryLabel).toBeNull();
  });
  it("noCommits → commit prompt with gitignore toggle", () => {
    const v = repoSetupView({ state: "noCommits", stageable: true, dirty: false }, "add");
    expect(v.kind).toBe("commit");
    expect(v.showGitignore).toBe(true);
    expect(v.primaryLabel).toMatch(/initial commit/i);
  });
  it("ready+dirty in add context → 'Add anyway' secondary", () => {
    const v = repoSetupView({ state: "ready", stageable: false, dirty: true }, "add");
    expect(v.kind).toBe("dirty");
    expect(v.secondaryLabel).toBe("Add anyway");
  });
  it("ready+dirty in spawn context → 'Spawn anyway' secondary", () => {
    const v = repoSetupView({ state: "ready", stageable: false, dirty: true }, "spawn");
    expect(v.secondaryLabel).toBe("Spawn anyway");
  });
  it("ready+clean → kind ready (no-op)", () => {
    const v = repoSetupView({ state: "ready", stageable: false, dirty: false }, "add");
    expect(v.kind).toBe("ready");
  });
});
