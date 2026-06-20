import { describe, expect, it } from "vitest";
import { AGENT_TYPES, agentColor } from "./agents";

describe("agents", () => {
  it("lists the three agent types", () => {
    expect(AGENT_TYPES.map((a) => a.id)).toEqual(["claude", "pi", "hermes"]);
  });
  it("maps each type to its badge color", () => {
    expect(agentColor("claude")).toBe("#fab387");
    expect(agentColor("pi")).toBe("#94e2d5");
    expect(agentColor("hermes")).toBe("#cba6f7");
  });
  it("falls back to a neutral color for unknown agents", () => {
    expect(agentColor("shell")).toBe("#a6adc8");
  });
});
