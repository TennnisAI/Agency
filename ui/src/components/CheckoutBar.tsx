import { useEffect, useState } from "react";
import { FileRoot, gitBranchInfo } from "../api";
import { checkoutTooltip, describeCheckout } from "../lib/checkout";
import { useRuns } from "../store/runs";
import { BranchIcon } from "./git/gitIcons";

// The branch is a caption, not a live counter like the ahead/behind pair in
// source control, so it re-reads slowly: it only has to catch up within a few
// seconds of a branch switch made in a terminal or by an agent.
const POLL_MS = 5000;

/**
 * Which working tree the Docs / Files tab is showing, the way source control
 * shows it: the agent worktree or project checkout the tree is rooted in, and
 * the branch checked out there. Both tabs follow the selected agent, so the bar
 * is what says where "here" is right now.
 *
 * It is also the way there: like the status bar's branch, clicking it opens
 * Source Control on the tree it names. The caller supplies that move, because
 * the tab lives above both views.
 */
export default function CheckoutBar({ root, projectId, projectName, onOpen }: {
  root: FileRoot;
  projectId: string;
  projectName: string;
  onOpen: () => void;
}) {
  const { runs } = useRuns();
  const checkout = describeCheckout({ root, projectId, projectName, runs });
  const { taskId } = checkout;
  // undefined until the first read lands (the bar shows the place meanwhile, so
  // the usual case doesn't shift); null once git has declined to answer.
  const [branch, setBranch] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    setBranch(undefined);
    const load = () => gitBranchInfo(taskId)
      .then((b) => { if (alive) setBranch(b.branch); })
      // A project without git (the workspace can decline it), or a run that has
      // gone away.
      .catch(() => { if (alive) setBranch(null); });
    load();
    const id = setInterval(load, POLL_MS);
    return () => { alive = false; clearInterval(id); };
  }, [taskId]);

  // No git at all: every agent in such a project works in the project folder
  // itself, so there is one place these files can be and no branch to confuse
  // it with. The bar has nothing to say.
  if (branch === null) return null;

  return (
    <button
      type="button"
      className="checkout-bar"
      title={checkoutTooltip(checkout, branch ?? null)}
      onClick={onOpen}
    >
      <span className="checkout-scope">{checkout.name}</span>
      {branch && (
        <span className="checkout-branch">
          <BranchIcon /> {branch}
        </span>
      )}
      <span className="checkout-kind">{checkout.kind}</span>
    </button>
  );
}
