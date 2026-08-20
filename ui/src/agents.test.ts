import { describe, expect, it } from "vitest";
import { AGENT_TYPES, INSTALL_COMMANDS, agentColor, filterModels, modelIdError, modelOptions, nextRaceModel, runListLabel } from "./agents";

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

describe("modelIdError", () => {
  it("accepts the shapes the supported CLIs take", () => {
    for (const id of ["opus", "claude-opus-5", "anthropic/claude-sonnet-5", "sonnet:high", "gpt-5.1"]) {
      expect(modelIdError(id)).toBeNull();
    }
    expect(modelIdError("  opus  ")).toBeNull();
  });

  it("refuses an id the agent would not read as a model", () => {
    expect(modelIdError("")).not.toBeNull();
    expect(modelIdError("   ")).not.toBeNull();
    // Would land in argv as another flag.
    expect(modelIdError("--dangerously-skip-permissions")).not.toBeNull();
    expect(modelIdError("opus; rm -rf /")).not.toBeNull();
    expect(modelIdError("a".repeat(129))).not.toBeNull();
  });
});

describe("modelOptions", () => {
  it("puts the vendor aliases first and drops duplicates", () => {
    expect(modelOptions(["opus", "sonnet"], ["sonnet", "claude-haiku-5"]))
      .toEqual(["opus", "sonnet", "claude-haiku-5"]);
    expect(modelOptions([], [])).toEqual([]);
  });
  it("keeps what the agent's CLI listed below the familiar ids, listed once", () => {
    expect(modelOptions([], ["openai/gpt-5.6"], ["openai/gpt-5.6", "anthropic/claude-opus-5"]))
      .toEqual(["openai/gpt-5.6", "anthropic/claude-opus-5"]);
    // An agent that was never probed is exactly what it was before.
    expect(modelOptions(["opus"], [])).toEqual(["opus"]);
  });
});

describe("nextRaceModel", () => {
  const suggested = ["opus", "sonnet", "haiku"];
  it("picks the first model the agent's other attempts are not on", () => {
    expect(nextRaceModel([null], suggested, [])).toBe("opus");
    expect(nextRaceModel(["opus"], suggested, [])).toBe("sonnet");
    expect(nextRaceModel(["sonnet", "opus"], suggested, [])).toBe("haiku");
  });
  it("falls back to models used before when the aliases run out", () => {
    expect(nextRaceModel(["opus", "sonnet", "haiku"], suggested, ["claude-opus-4-1"]))
      .toBe("claude-opus-4-1");
  });
  it("returns null when the agent has nothing left to offer", () => {
    expect(nextRaceModel(["opus"], ["opus"], [])).toBeNull();
    expect(nextRaceModel([], [], [])).toBeNull();
  });
});

describe("filterModels", () => {
  const listed = ["anthropic/claude-opus-5", "openai/gpt-5.6", "openai/gpt-5.6-pro"];
  it("matches anywhere in the id, whatever the case", () => {
    expect(filterModels(listed, "opus")).toEqual(["anthropic/claude-opus-5"]);
    expect(filterModels(listed, "GPT-5.6")).toEqual(["openai/gpt-5.6", "openai/gpt-5.6-pro"]);
  });
  it("shows everything until something is typed, and nothing that doesn't match", () => {
    expect(filterModels(listed, "")).toEqual(listed);
    expect(filterModels(listed, "   ")).toEqual(listed);
    expect(filterModels(listed, "gemini")).toEqual([]);
  });
});
