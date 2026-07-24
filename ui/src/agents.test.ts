import { describe, expect, it } from "vitest";
import { AGENT_TYPES, INSTALL_COMMANDS, agentColor } from "./agents";

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
