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
  // The selected run, or null in the grid — where the branch on show is the one
  // checked out in the project itself.
  runId: string | null;
  projectId: string | null;
  // Source Control for whatever is selected (the run's worktree, or the
  // checkout when nothing is).
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
      // branch to report.
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

  // In a run, the run's own branches; in the grid, the checkout's branch.
  const branch = runId ? run?.branch ?? "" : checkout?.branch ?? "";
  const base = runId ? run?.base ?? "" : "";
  return (
    <footer className="statusbar">
      <span className="statusbar-left">
        <span>{projectName ?? "no project"}</span>
        <span className="statusbar-git">
          {branch && (
            <button
              className="statusbar-branch"
              title={runId
                ? "Source branch → merge target. Opens Source Control."
                : "The branch checked out in your project. Opens Source Control."}
              onClick={onOpenSource}
            >
              ⎇ {branch}{base && base !== branch ? ` → ${base}` : ""}
            </button>
          )}
        </span>
      </span>
      {/* Centred, apart from both the selection's branch and the hints, so it
          reads as its own standing fact rather than part of either. */}
      <span className="statusbar-mid">
        {checkout && <CheckoutSegment status={checkout} onOpen={onOpenCheckout} />}
      </span>
      <span className="statusbar-right">
        <span className="kbd-hints">
          {shortcutLabel("⌘N")} new · {shortcutLabel("⌘D")} source · {shortcutLabel("⌘↵")} approve · {shortcutLabel("⌘,")} settings
        </span>
      </span>
    </footer>
  );
}

/**
 * The project checkout's branch and what it has to push and pull, shown at all
 * times and in the same spot, whichever agent is selected (AGE-240). Working
 * mostly in agent worktrees, the checkout is out of sight: merges land on it and
 * sit unpushed, and switching to another computer means doing the work twice.
 * This is the standing reminder, so it is a separate segment rather than a
 * variant of the branch on the left, which follows the selection.
 */
function CheckoutSegment({ status, onOpen }: { status: CheckoutStatus; onOpen: () => void }) {
  const unpushed = status.kind === "tracked" && status.ahead > 0;
  return (
    <button
      className={`statusbar-checkout${unpushed ? " unpushed" : ""}`}
      title={checkoutStatusTooltip(status)}
      onClick={onOpen}
    >
      <span className="statusbar-checkout-label">checkout</span>
      <span className="statusbar-checkout-branch">⎇ {status.branch}</span>
      <CheckoutCounts status={status} />
    </button>
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
