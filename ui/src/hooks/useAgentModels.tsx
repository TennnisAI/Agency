import { useCallback, useEffect, useState } from "react";
import { AgentModelInfo, listAgentModels } from "../api";

/**
 * The model picker's data for every enabled agent, keyed by agent id, plus the
 * model each one was last launched on.
 *
 * Reloaded on demand rather than cached globally: the selection moves whenever
 * a run starts, and a menu that reopened on a stale model would offer to repeat
 * a choice the user has already moved on from. The call reads settings and
 * static data only, so it is cheap enough to make every time a menu opens.
 */
export function useAgentModels() {
  const [models, setModels] = useState<Record<string, AgentModelInfo>>({});

  const reload = useCallback(() => {
    listAgentModels()
      .then((list) => setModels(Object.fromEntries(list.map((m) => [m.agent, m]))))
      // Leave the last good map standing: without it the pickers vanish, and
      // an agent silently losing its model control is worse than a stale list.
      .catch(() => {});
  }, []);

  useEffect(() => { reload(); }, [reload]);

  /** The model to start `agent` on by default: the one it last ran on. */
  const remembered = useCallback(
    (agent: string) => models[agent]?.selected ?? null,
    [models],
  );

  return { models, reload, remembered };
}
