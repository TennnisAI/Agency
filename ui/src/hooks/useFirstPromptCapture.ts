import { useMemo } from "react";
import { setRunTitle } from "../api";
import { useRuns } from "../store/runs";

/**
 * The handler `FocusTerminal` calls with the first line the user types into a
 * promptless agent, or `undefined` once the run has a title — which is also how
 * the capture is switched off.
 *
 * That line names the run: it fills the empty title and, when the branch is
 * still the empty-prompt `agent/agent-<suffix>` fallback, renames that to match
 * (AGE-183, see `set_run_title`). Both agent panes want this, so it lives here
 * rather than twice. Whether the pane is one that should capture at all (the
 * primary tab, an agent rather than a terminal) is the caller's own condition.
 */
export function useFirstPromptCapture(
  runId: string | undefined,
  title: string | null | undefined,
): ((line: string) => void) | undefined {
  const { refreshRuns } = useRuns();
  const capture = !!runId && !title;
  return useMemo(
    () =>
      capture && runId
        ? (line: string) => {
            // Refresh so the branch chip picks up an AGE-183 rename instead of
            // waiting out the run poll.
            setRunTitle(runId, line)
              .then(() => refreshRuns())
              .catch(() => {});
          }
        : undefined,
    [capture, runId, refreshRuns],
  );
}
