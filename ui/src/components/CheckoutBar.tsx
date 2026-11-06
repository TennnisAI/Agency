import { Fragment, useEffect, useRef, useState } from "react";
import { FileRoot, gitBranchInfo } from "../api";
import { CheckoutChoice, branchTooltip, checkoutChoices, checkoutTooltip, describeCheckout, isCurrentChoice } from "../lib/checkout";
import { useRuns } from "../store/runs";
import { useMenuAnchor } from "../hooks/useMenuAnchor";
import { useDismissOnResize } from "../hooks/useDismissOnResize";
import { BranchIcon } from "./git/gitIcons";

// The branch is a caption, not a live counter like the ahead/behind pair in
// source control, so it re-reads slowly: it only has to catch up within a few
// seconds of a branch switch made in a terminal or by an agent.
const POLL_MS = 5000;

/**
 * Which working tree the Docs, Files or Source Control tab is showing: the
 * agent worktree or project checkout the view is rooted in, and the branch
 * checked out there. All three follow the selected agent, so the bar is what
 * says where "here" is right now.
 *
 * It is also the way to somewhere else (AGE-277). The name opens a menu of the
 * project checkout and every agent worktree, and a pick re-roots the view in
 * place. Before this the only way from an agent's files to the checkout's was
 * the Agents tab's Grid button and back, which nothing on the Files tab
 * mentioned. The branch opens Source Control on the tree it names, like the
 * status bar's branch; the caller supplies that move, because the tab lives
 * above the views. Source Control itself passes no `onOpen` and shows no
 * branch here, since its own branch bar sits right below.
 */
export default function CheckoutBar({ root, projectId, projectName, onOpen }: {
  root: FileRoot;
  projectId: string;
  projectName: string;
  onOpen?: () => void;
}) {
  const { runs, setFocusedRun, setView } = useRuns();
  const checkout = describeCheckout({ root, projectId, projectName, runs });
  const { taskId } = checkout;
  // undefined until the first read lands (the bar shows the place meanwhile, so
  // the usual case doesn't shift); null once git has declined to answer.
  const [branch, setBranch] = useState<string | null | undefined>(undefined);
  const [open, setOpen] = useState(false);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const coords = useMenuAnchor(open ? rect : null, menuRef);

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

  useEffect(() => { setOpen(false); }, [taskId]);
  useDismissOnResize(open, () => setOpen(false));
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      setOpen(false);
      btnRef.current?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  // No git at all: every agent in such a project works in the project folder
  // itself, so there is one place these files can be and no branch to confuse
  // it with. The bar has nothing to say.
  if (branch === null) return null;

  const toggle = () => {
    if (open) { setOpen(false); return; }
    setRect(btnRef.current?.getBoundingClientRect() ?? null);
    setOpen(true);
  };

  // The same two moves the rest of the app makes. The checkout is the grid's
  // tree, as the status bar's checkout segment has it: the run stays focused,
  // so picking it here again, or the Agents tab's Focus button, goes straight
  // back. A worktree is its run focused, which is what roots these views there.
  // The tab stays put either way: being moved off Files to change which files
  // it shows is the detour this replaces.
  const pick = (choice: CheckoutChoice) => {
    setOpen(false);
    if (choice.runId) {
      setFocusedRun(choice.runId);
      setView("focus");
    } else {
      setView("grid");
    }
  };

  const choices = checkoutChoices(projectName, runs);

  return (
    <div className="checkout-bar">
      <button
        ref={btnRef}
        type="button"
        className={`checkout-switch${open ? " on" : ""}`}
        title={checkoutTooltip(checkout)}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={toggle}
      >
        <span className="checkout-scope">{checkout.name}</span>
        <span className="checkout-kind">{checkout.kind}</span>
        <span className="checkout-chev" aria-hidden>▾</span>
      </button>
      {branch && onOpen && (
        <button type="button" className="checkout-branch" title={branchTooltip(checkout, branch)} onClick={onOpen}>
          <BranchIcon /> {branch}
        </button>
      )}

      {open && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div
            ref={menuRef}
            role="menu"
            className="agent-menu checkout-menu"
            style={{ position: "fixed", ...coords }}
          >
            {choices.map((c, i) => {
              const current = isCurrentChoice(checkout, c);
              return (
                <Fragment key={c.runId ?? "checkout"}>
                  {i === 1 && <div className="agent-menu-sep" />}
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={current}
                    className={current ? "on" : ""}
                    autoFocus={current}
                    onClick={() => pick(c)}
                  >
                    <span className="checkout-menu-name">{c.name}</span>
                    <span className="checkout-menu-sub">
                      {c.kind === "checkout" ? "project checkout" : c.branch || "worktree"}
                    </span>
                    {current && <span className="checkout-menu-tick" aria-hidden>✓</span>}
                  </button>
                </Fragment>
              );
            })}
            {choices.length === 1 && (
              <div className="checkout-menu-none">No agent has a worktree of its own yet.</div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
