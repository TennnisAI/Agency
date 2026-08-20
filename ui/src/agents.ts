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
  { id: "gemini", label: "Gemini CLI" },
  { id: "kimi", label: "Kimi Code" },
  { id: "crush", label: "Crush" },
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
  gemini: "npm install -g @google/gemini-cli",
  kimi: "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash",
  crush: "npm install -g @charmland/crush",
};

const COLORS: Record<string, string> = {
  claude: "#fab387",
  pi: "#94e2d5",
  hermes: "#cba6f7",
  gemini: "#89b4fa",
  kimi: "#f5c2e7",
  crush: "#a6e3a1",
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

// The palette as bare accent names ("blue", "mauve", …) — the form stored on a
// project and offered in the color picker. Must stay in step with
// PROJECT_COLORS in registry.rs, which is what validates a pick.
export const PROJECT_COLOR_NAMES = PROJECT_PALETTE.map((v) => v.slice(2));

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

// One-line label for a run in a picker list — the focus rail and the docs/files
// agents panel share it, so a run reads the same wherever it's listed.
export function runListLabel(
  run: Pick<RunInfo, "kind" | "title" | "prompt" | "branch" | "agent" | "loopConfig" | "raceId">,
): string {
  if (run.kind === "terminal") return `≳ ${run.title || "terminal"}`;
  return `${run.loopConfig ? "⟳ " : run.raceId ? "∥ " : ""}${run.agent}: ${runName(run)}`;
}

// Mirror of `sanitize_model` in agent_catalog.rs, so a typed model id is
// refused in the field rather than after a spawn round-trip. The backend still
// checks: this is the message, not the guard.
const MODEL_MAX = 128;
export function modelIdError(raw: string): string | null {
  const model = raw.trim();
  if (!model) return "Type a model id, or pick the agent's default.";
  if (model.length > MODEL_MAX) return `Model ids are at most ${MODEL_MAX} characters.`;
  if (model.startsWith("-")) return "A model id can't start with '-'; the agent would read it as a flag.";
  if (!/^[A-Za-z0-9._:/@+-]+$/.test(model)) return "Letters, digits and - _ . / : @ + only.";
  return null;
}

// What the model picker offers for an agent: the vendor's stable aliases first,
// then anything used before, then whatever the agent's own CLI said it has when
// asked (AGE-117). Deduped, so a model that is two of those three appears once,
// and ordered so the short familiar list stays above the long probed one.
export function modelOptions(
  suggested: string[],
  recent: string[],
  listed: string[] = [],
): string[] {
  return [...new Set([...suggested, ...recent, ...listed])];
}

// The model to seed an extra race attempt with: the first thing this agent
// could run that its other attempts are not already on. Racing one agent
// against itself is a model comparison (AGE-118), so two rows on the same model
// would be a race against nothing but the agent's own nondeterminism. Null when
// the agent has nothing left to offer, which starts that row on its own default
// and leaves the picker to sort it out.
export function nextRaceModel(
  used: (string | null)[],
  suggested: string[],
  recent: string[],
): string | null {
  const taken = new Set(used);
  return modelOptions(suggested, recent).find((m) => !taken.has(m)) ?? null;
}

// The picker's field is a filter as well as an entry box: a probed list runs to
// 200 models for some agents, which is unreadable without one. Matched on a
// plain substring, case-insensitively, because model ids are typed from memory
// ("opus", "5.6") rather than recognised in full.
export function filterModels(options: string[], typed: string): string[] {
  const needle = typed.trim().toLowerCase();
  if (!needle) return options;
  return options.filter((m) => m.toLowerCase().includes(needle));
}
