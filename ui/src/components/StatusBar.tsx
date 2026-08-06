import { useEffect, useState } from "react";
import { gitBranchInfo, projectTarget, runBranches } from "../api";

// What the project's own checkout looks like from here: the branch you have
// checked out, and how many commits on it origin hasn't seen. The count is the
// standing answer to "did I push that merge?" (AGE-64) — it is shown whichever
// working tree you happen to be standing in.
type Checkout = { branch: string; unpushed: number };

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
  // agent, this is the way out to the branch the unpushed commits are on.
  onOpenCheckout: () => void;
}) {
  const [run, setRun] = useState<{ branch: string; base: string } | null>(null);
  const [checkout, setCheckout] = useState<Checkout | null>(null);

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
      .then((b) => {
        // Counted only against a real upstream: with none, git's "ahead" is
        // measured from the fork point and says nothing about origin.
        if (alive) setCheckout({ branch: b.branch, unpushed: b.upstream ? b.ahead : 0 });
      })
      // A project without git (the workspace can decline it) simply has no
      // branch to report.
      .catch(() => { if (alive) setCheckout(null); });
    load();
    const id = setInterval(load, 2000);
    return () => { alive = false; clearInterval(id); };
  }, [projectId]);

  // In a run, the run's own branches; in the grid, the checkout's branch.
  const branch = runId ? run?.branch ?? "" : checkout?.branch ?? "";
  const base = runId ? run?.base ?? "" : "";
  const unpushed = checkout?.unpushed ?? 0;
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
          {unpushed > 0 && checkout && (
            <button
              className="statusbar-unpushed"
              title={`${unpushed} commit${unpushed === 1 ? "" : "s"} on ${checkout.branch} that origin doesn't have. Opens Source Control for your checkout, where Push is.`}
              onClick={onOpenCheckout}
            >
              {/* Named only when it isn't the branch already on show, so a
                  merged-but-unpushed base is never mistaken for the agent's. */}
              {runId ? `${checkout.branch} ` : ""}{unpushed}↑
            </button>
          )}
        </span>
      </span>
      <span className="kbd-hints">⌘N new · ⌘G source · ⌘↵ approve · ⌘, settings</span>
    </footer>
  );
}
