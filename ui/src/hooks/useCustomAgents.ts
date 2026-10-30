import { useCallback, useEffect, useState } from "react";
import { CustomAgentInfo, CustomAgents, listCustomAgents } from "../api";

/**
 * How a spawn picker names an agent. A project's agents arrive through git, so
 * the one that came from the repo says so, and it can't pass for one of yours
 * with the same name. A prefix, not a suffix: pills truncate at 170px, so a long
 * name would have cut "(this project)" off the end.
 */
export const customAgentLabel = (a: CustomAgentInfo) =>
  a.scope === "project" ? `Project: ${a.name}` : a.name;

/**
 * Which project's agents a dialog lists: the issue's when there is one, since
 * that is where the backend looks the pick up, whichever project is selected.
 */
export const customAgentsProject = (issue: { projectId: string } | undefined, selected: string | null) =>
  issue?.projectId ?? selected;

/**
 * The race attempts left once a custom agent is picked. The backend refuses a
 * whole race over one harness that can't take it, so those are unticked up front.
 */
export const attemptsForPick = <T extends { agent: string }>(
  attempts: T[],
  selection: string,
  takes: (harness: string) => boolean,
) => (selection ? attempts.filter((a) => takes(a.agent)) : attempts);

/**
 * What a single-harness spawn sends. Derived rather than cleared, so trying
 * another harness and coming back keeps the pick, and one that can't take it is
 * never sent it.
 */
export const pickFor = (harness: string, selection: string, takes: (harness: string) => boolean) =>
  takes(harness) ? selection : "";

/**
 * The custom agents a spawn in this project can pick, and the harnesses that can
 * be given one. Reloaded on demand, as the models are: an agent written moments
 * ago in Settings should be in the menu that opens next. A failed read leaves
 * the last list standing, and the spawn itself is what reports a bad pick.
 */
export function useCustomAgents(projectId: string | null) {
  const [data, setData] = useState<CustomAgents>({ agents: [], harnesses: [], skipped: [] });
  const reload = useCallback(() => {
    listCustomAgents(projectId).then(setData).catch(() => {});
  }, [projectId]);
  useEffect(reload, [reload]);
  const takes = useCallback((harness: string) => data.harnesses.includes(harness), [data.harnesses]);
  return { agents: data.agents, takes, reload };
}
