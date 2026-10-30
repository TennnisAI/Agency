import { useCallback, useEffect, useState } from "react";
import { CustomAgents, listCustomAgents } from "../api";

/**
 * The custom agents a spawn in this project can pick, and the harnesses that can
 * be given one. Reloaded on demand, as the models are: an agent written moments
 * ago in Settings should be in the menu that opens next. A failed read leaves
 * the last list standing, and the spawn itself is what reports a bad pick.
 */
export function useCustomAgents(projectId: string | null) {
  const [data, setData] = useState<CustomAgents>({ agents: [], harnesses: [] });
  const reload = useCallback(() => {
    listCustomAgents(projectId).then(setData).catch(() => {});
  }, [projectId]);
  useEffect(reload, [reload]);
  const takes = useCallback((harness: string) => data.harnesses.includes(harness), [data.harnesses]);
  return { agents: data.agents, takes, reload };
}
