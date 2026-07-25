import { getSettings, listProfiles, listProjects } from "../api";

// The agent a default "start" action spawns, shared by the menu-bar New Agent
// flow and the issue board's Start-agent button so the two can't drift: the
// Settings default if set, else the project's last-used agent, else the first
// enabled profile (or "claude" if somehow none are enabled).
export async function pickDefaultAgent(
  projectId: string | null,
  fallback?: string | null,
): Promise<string> {
  const chosen = await getSettings().then((s) => s.defaultAgent).catch(() => null);
  if (chosen) return chosen;
  const projects = await listProjects().catch(() => null);
  const current = projects?.find((p) => p.id === projectId);
  if (current?.default_agent) return current.default_agent;
  if (fallback) return fallback;
  const profiles = await listProfiles().catch(() => []);
  return profiles[0]?.name ?? "claude";
}
