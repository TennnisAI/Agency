import type { RunInfo } from "./api";

export interface AgentType {
  id: string;
  label: string;
}

export const AGENT_TYPES: AgentType[] = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex" },
  { id: "pi", label: "Pi" },
  { id: "opencode", label: "OpenCode" },
  { id: "copilot", label: "Copilot CLI" },
  { id: "cursor", label: "Cursor" },
  { id: "hermes", label: "Hermes" },
];

// Install one-liners for the preconfigured agents, used when a spawn is
// attempted and the profile's command isn't on PATH. Agents without an entry
// (e.g. hermes) get a "configure it in Settings" hint instead of an install
// offer. Runs in the user's login shell, inside an Agency terminal.
export const INSTALL_COMMANDS: Record<string, string> = {
  claude: "npm install -g @anthropic-ai/claude-code",
  codex: "npm install -g @openai/codex",
  pi: "npm install -g --ignore-scripts @earendil-works/pi-coding-agent",
  opencode: "npm install -g opencode-ai",
  copilot: "npm install -g @github/copilot",
  cursor: "curl https://cursor.com/install -fsS | bash",
};

const COLORS: Record<string, string> = {
  claude: "#fab387",
  pi: "#94e2d5",
  hermes: "#cba6f7",
};

export function agentColor(name: string): string {
  return COLORS[name] ?? "#a6adc8";
}

// Palette of theme accent vars used to give each project a distinct, stable
// color. Drawing from CSS vars (rather than fixed hex) keeps project colors in
// step with the active theme.
const PROJECT_PALETTE = [
  "--blue", "--mauve", "--green", "--peach", "--teal",
  "--pink", "--yellow", "--lav", "--red",
] as const;

// Deterministic accent for a project, derived from its id so the color is
// stable across reloads and distinct between adjacent projects.
export function projectColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return `var(${PROJECT_PALETTE[h % PROJECT_PALETTE.length]})`;
}

// Preferred accent: the color assigned at add time (least-used palette entry,
// so new projects never collide while unused colors remain). The id-hash is
// only the fallback for rows predating the color column.
export function projectAccent(p: { id: string; color?: string | null }): string {
  return p.color ? `var(--${p.color})` : projectColor(p.id);
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
