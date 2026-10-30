import { describe, it, expect } from "vitest";
import { gitlessOpts } from "./useSpawnAgent";

describe("gitlessOpts", () => {
  it("keeps the picked agent and model", () => {
    const opts = gitlessOpts({ customAgent: "user:reviewer", model: "opus", worktree: true });
    expect(opts.customAgent).toBe("user:reviewer");
    expect(opts.model).toBe("opus");
  });

  it("always stays in the folder", () => {
    expect(gitlessOpts({ worktree: true }).worktree).toBe(false);
    expect(gitlessOpts().worktree).toBe(false);
  });
});
