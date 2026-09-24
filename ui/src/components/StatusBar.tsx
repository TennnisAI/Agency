import { useEffect, useState } from "react";
import { gitAutoFetch, gitBranchInfo, projectTarget, runBranches } from "../api";
import { CheckoutStatus, checkoutStatus, checkoutStatusTooltip } from "../lib/checkoutStatus";
import { shortcutLabel } from "../lib/platform";

export default function StatusBar({
  projectName,
  runId,
  projectId,
  onOpenSource,
  onOpenCheckout,
}: {
  projectName: string | null;
  // The selected run, or null in the grid, where the checkout is the only
  // working tree on show.
  runId: string | null;
  projectId: string | null;
  // Source Control for the selected run's worktree.
  onOpenSource: () => void;
  // Source Control for the project's checkout specifically: from inside an
  // agent, this is the way out to the branch Push and Pull act on.
  onOpenCheckout: () => void;
}) {
  const [run, setRun] = useState<{ branch: string; base: string } | null>(null);
  const [checkout, setCheckout] = useState<CheckoutStatus | null>(null);

  useEffect(() => {
    setRun(null);
    if (!runId) return;
    let alive = true;
    const load = () => runBranches(runId)
      .then((b) => { if (alive) setRun(b); })
      .catch(() => { if (alive) setRun(null); });
    load();
    const id = setInterval(load, 2000);
    return () => { alive = false; clearInterval(id); };
  }, [runId]);

  useEffect(() => {
    setCheckout(null);
    if (!projectId) return;
    let alive = true;
    const load = () => gitBranchInfo(projectTarget(projectId))
      .then((b) => { if (alive) setCheckout(checkoutStatus(b)); })
      // A project without git (the workspace can decline it) simply has no
      // checkout to report.
      .catch(() => { if (alive) setCheckout(null); });
    load();
    const id = setInterval(load, 2000);
    return () => { alive = false; clearInterval(id); };
  }, [projectId]);

  // The poll above re-reads local refs only, so the incoming count is whatever
  // it was at the last fetch. The background sweep runs every five minutes;
  // this asks for a fresher one when the window comes back, which is when the
  // bar is about to be read. Throttled and silent backend-side, as it is for
  // Source Control.
  useEffect(() => {
    if (!projectId) return;
    const fetchNow = () => { gitAutoFetch(projectTarget(projectId)).catch(() => {}); };
    fetchNow();
    window.addEventListener("focus", fetchNow);
    return () => window.removeEventListener("focus", fetchNow);
  }, [projectId]);

  return (
    <footer className="statusbar">
      <span className="statusbar-left">
        <span>{projectName ?? "no project"}</span>
        <span className="statusbar-git">
          {/* First and always, so it sits in the same spot whichever agent is
              selected: the checkout is where merges land. */}
          {checkout && (
            <button
              className="statusbar-branch statusbar-checkout"
              title={checkoutStatusTooltip(checkout)}
              onClick={onOpenCheckout}
            >
              ⎇ {checkout.branch}
              <CheckoutCounts status={checkout} />
            </button>
          )}
          {runId && run?.branch && (
            <button
              className="statusbar-branch statusbar-run"
              title="The agent's branch → its merge target. Opens Source Control for the agent."
              onClick={onOpenSource}
            >
              ⎇ {run.branch}{run.base && run.base !== run.branch ? ` → ${run.base}` : ""}
            </button>
          )}
        </span>
      </span>
      <span className="kbd-hints">
        {shortcutLabel("⌘N")} new · {shortcutLabel("⌘D")} source · {shortcutLabel("⌘↵")} approve · {shortcutLabel("⌘,")} settings
      </span>
    </footer>
  );
}

function CheckoutCounts({ status }: { status: CheckoutStatus }) {
  if (status.kind === "local") return <span className="statusbar-sync-note">no remote</span>;
  if (status.kind === "unpublished") return <span className="statusbar-sync-note">unpublished</span>;
  const { ahead, behind } = status;
  if (ahead === 0 && behind === 0) return <span className="statusbar-sync-note">✓</span>;
  return (
    <>
      {behind > 0 && <span className="statusbar-behind">{behind}↓</span>}
      {ahead > 0 && <span className="statusbar-ahead">{ahead}↑</span>}
    </>
  );
}
