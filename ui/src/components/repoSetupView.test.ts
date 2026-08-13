import { describe, it, expect } from "vitest";
import { repoSetupView } from "./repoSetupView";

describe("repoSetupView", () => {
  it("notARepo in add context → init prompt with a way past it", () => {
    const v = repoSetupView({ state: "notARepo", stageable: false, dirty: false }, "add");
    expect(v.kind).toBe("init");
    expect(v.title).toMatch(/Set up/i);
    expect(v.secondaryLabel).toBe("Add without git");
  });
  it("notARepo in spawn context → no secondary (spawn never opens this)", () => {
    const v = repoSetupView({ state: "notARepo", stageable: false, dirty: false }, "spawn");
    expect(v.kind).toBe("init");
    expect(v.secondaryLabel).toBeNull();
  });
  it("noCommits → initial commit prompt", () => {
    const v = repoSetupView({ state: "noCommits", stageable: true, dirty: false }, "add");
    expect(v.kind).toBe("commit");
    expect(v.primaryLabel).toMatch(/initial commit/i);
  });
  // Reachable by initializing from the notARepo step and then backing out of
  // the commit: without a secondary here that folder could never be added.
  it("noCommits in add context → 'Add anyway' secondary", () => {
    const v = repoSetupView({ state: "noCommits", stageable: true, dirty: false }, "add");
    expect(v.secondaryLabel).toBe("Add anyway");
  });
  it("noCommits in spawn context → no secondary (a worktree needs a commit)", () => {
    const v = repoSetupView({ state: "noCommits", stageable: true, dirty: false }, "spawn");
    expect(v.secondaryLabel).toBeNull();
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
