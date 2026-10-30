import { describe, it, expect } from "vitest";
import { attemptsForPick, customAgentsProject, pickFor } from "./useCustomAgents";

const takes = (harness: string) => harness === "claude" || harness === "codex";

describe("customAgentsProject", () => {
  it("lists the issue's project, whichever one is selected", () => {
    expect(customAgentsProject({ projectId: "b" }, "a")).toBe("b");
  });

  it("falls back to the selected project without an issue", () => {
    expect(customAgentsProject(undefined, "a")).toBe("a");
    expect(customAgentsProject(undefined, null)).toBe(null);
  });
});

describe("attemptsForPick", () => {
  const attempts = [{ agent: "claude" }, { agent: "gemini" }, { agent: "codex" }];

  it("unticks a harness that can't take the agent", () => {
    expect(attemptsForPick(attempts, "user:reviewer", takes)).toEqual([{ agent: "claude" }, { agent: "codex" }]);
  });

  it("leaves the race alone when no agent is picked", () => {
    expect(attemptsForPick(attempts, "", takes)).toEqual(attempts);
  });
});

describe("pickFor", () => {
  it("sends the pick to a harness that can take it", () => {
    expect(pickFor("claude", "user:reviewer", takes)).toBe("user:reviewer");
  });

  it("sends nothing to one that can't", () => {
    expect(pickFor("gemini", "user:reviewer", takes)).toBe("");
  });
});
