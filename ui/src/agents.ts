import type { RunInfo } from "./api";

export interface AgentType {
  id: string;
  label: string;
}

export const AGENT_TYPES: AgentType[] = [
  { id: "claude", label: "Claude Code" },
  { id: "pi", label: "Pi" },
  { id: "hermes", label: "Hermes" },
];

const COLORS: Record<string, string> = {
  claude: "#fab387",
  pi: "#94e2d5",
  hermes: "#cba6f7",
};

export function agentColor(name: string): string {
  return COLORS[name] ?? "#a6adc8";
}

// Friendly label for an agent profile name. Built-ins get a nicer display name;
// custom profiles (e.g. "cursor") fall back to the raw name the user defined.
export function agentLabel(name: string): string {
  return AGENT_TYPES.find((a) => a.id === name)?.label ?? name;
}

// Display name precedence: generated title, else the original prompt, else branch.
export function runName(run: Pick<RunInfo, "title" | "prompt" | "branch">): string {
  return run.title || run.prompt || run.branch;
}
