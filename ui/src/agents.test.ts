import { describe, expect, it } from "vitest";
import { AGENT_TYPES, INSTALL_COMMANDS, agentColor, runListLabel } from "./agents";

describe("agents", () => {
  it("lists the preconfigured agent types", () => {
    expect(AGENT_TYPES.map((a) => a.id)).toEqual([
      "claude", "codex", "pi", "opencode", "copilot", "cursor", "hermes",
      "gemini", "kimi", "crush",
    ]);
  });
  it("has an install command for every preconfigured agent except hermes", () => {
    const withInstall = AGENT_TYPES.filter((a) => a.id !== "hermes").map((a) => a.id);
    for (const id of withInstall) expect(INSTALL_COMMANDS[id], id).toBeTruthy();
    expect(INSTALL_COMMANDS.hermes).toBeUndefined();
  });
  it("maps each type to its badge color", () => {
    expect(agentColor("claude")).toBe("#fab387");
    expect(agentColor("pi")).toBe("#94e2d5");
    expect(agentColor("hermes")).toBe("#cba6f7");
    expect(agentColor("gemini")).toBe("#89b4fa");
    expect(agentColor("kimi")).toBe("#f5c2e7");
    expect(agentColor("crush")).toBe("#a6e3a1");
  });
  it("falls back to a neutral color for unknown agents", () => {
    expect(agentColor("shell")).toBe("#a6adc8");
  });
});

describe("runListLabel", () => {
  const base = {
    kind: "agent" as const, title: null, prompt: "", branch: "agent/x",
    agent: "claude", loopConfig: null, raceId: null,
  };

  it("names a terminal by its title, falling back to 'terminal'", () => {
    expect(runListLabel({ ...base, kind: "terminal", title: "build" })).toBe("≳ build");
    expect(runListLabel({ ...base, kind: "terminal" })).toBe("≳ terminal");
  });
  it("shows an agent as agent: name, preferring title over prompt over branch", () => {
    expect(runListLabel({ ...base, title: "Fix login" })).toBe("claude: Fix login");
    expect(runListLabel({ ...base, prompt: "fix the login" })).toBe("claude: fix the login");
    expect(runListLabel(base)).toBe("claude: agent/x");
  });
  it("marks looping and racing runs", () => {
    expect(runListLabel({ ...base, loopConfig: { maxAttempts: 3 } as never })).toBe("⟳ claude: agent/x");
    expect(runListLabel({ ...base, raceId: "r1" })).toBe("∥ claude: agent/x");
  });
});
