import { useState } from "react";
import {
  BranchInfo, gitCheckoutBranch, gitCreateBranch, gitFetch, gitListBranches,
  gitPull, gitPullRebase, gitStashPop, gitStashPush,
} from "../../api";
import Menu, { MenuEntry, menuAt } from "./Menu";
import { BranchIcon, StashIcon, CloudIcon } from "./gitIcons";
import ConfirmDialog from "../ConfirmDialog";
import PromptDialog from "../PromptDialog";

export default function BranchBar({ taskId, info, busy = false, onAct, onPush, onForcePush, onRefresh, onUndoCommit }: {
  taskId: string;
  info: BranchInfo | null;
  busy?: boolean;
  onAct: (fn: () => Promise<unknown>, label?: string) => Promise<boolean>;
  // Push has its own handler (streams progress) rather than going through onAct.
  onPush: () => void;
  // So does force push: it re-uploads the whole branch after a rebase, so it
  // needs the same progress bar and the same Cancel on it.
  onForcePush: () => void;
  onRefresh: () => void;
  onUndoCommit: () => void;
}) {
  // Menu items are derived at render time from this state, so an item that
  // reopens another menu (Checkout To…) always sees live coordinates.
  const [menu, setMenu] = useState<
    | { x: number; y: number; view: "overflow" }
    | { x: number; y: number; view: "picker"; branches: { current: string; branches: string[] } | null }
    | null
  >(null);
  const [confirmForce, setConfirmForce] = useState(false);
  const [stashPrompt, setStashPrompt] = useState(false);
  const [branchPrompt, setBranchPrompt] = useState(false);

  if (!info) return null;

  const openBranchPicker = async (at: { x: number; y: number }) => {
    // Load the branch list first so the menu opens complete, not half-filled.
    try {
      const pb = await gitListBranches(taskId);
      setMenu({ ...at, view: "picker", branches: pb });
    } catch {
      setMenu({ ...at, view: "picker", branches: null });
    }
  };

  const pickerItems = (pb: { current: string; branches: string[] } | null): MenuEntry[] => pb ? [
    { kind: "header", label: "Switch branch" },
    ...pb.branches.map((b): MenuEntry => ({
      label: b,
      glyph: <BranchIcon />,
      hint: b === pb.current ? "current" : undefined,
      disabled: b === pb.current,
      onClick: () => onAct(() => gitCheckoutBranch(taskId, b), `Switched to ${b}`),
    })),
    { kind: "separator" },
    { label: "Create New Branch…", glyph: "+", onClick: () => setBranchPrompt(true) },
  ] : [{ kind: "header", label: "Branches unavailable" }];

  const overflowItems = (at: { x: number; y: number }): MenuEntry[] => [
    { label: "Pull", disabled: !info.upstream, onClick: () => onAct(() => gitPull(taskId), "Pulled") },
    { label: "Pull (Rebase)", disabled: !info.upstream, onClick: () => onAct(() => gitPullRebase(taskId), "Pulled (rebase)") },
    { label: "Push", disabled: !info.hasRemote, onClick: onPush },
    { label: "Push (Force)…", disabled: !info.upstream, danger: true, onClick: () => setConfirmForce(true) },
    { label: "Fetch", disabled: !info.hasRemote, onClick: () => onAct(() => gitFetch(taskId), "Fetched") },
    { kind: "separator" },
    { label: "Checkout To…", glyph: <BranchIcon />, onClick: () => openBranchPicker(at) },
    { label: "Create Branch…", glyph: <BranchIcon />, onClick: () => setBranchPrompt(true) },
    { kind: "separator" },
    { label: "Stash All Changes…", glyph: <StashIcon />, onClick: () => setStashPrompt(true) },
    { label: "Pop Latest Stash", glyph: <StashIcon />, onClick: () => onAct(() => gitStashPop(taskId, 0), "Stash popped") },
    { kind: "separator" },
    { label: "Undo Last Commit", onClick: onUndoCommit },
  ];

  return (
    <div className="git-branchbar">
      <button className="git-branch" title="Checkout branch…" disabled={busy}
        onClick={(e) => openBranchPicker(menuAt(e))}>
        <BranchIcon size={12} /> {info.branch}
      </button>
      {info.upstream && <span className="git-upstream" title={`upstream: ${info.upstream}`}><CloudIcon /></span>}
      {(info.ahead > 0 || info.behind > 0) && (
        <span className="git-aheadbehind">
          {info.behind > 0 && <span title={`${info.behind} behind upstream`}>{info.behind}↓</span>}
          {/* Without an upstream, ahead is measured from the branch's fork point
              instead — the commits this branch alone adds. With no base either
              (a repo that has no origin at all) it's the whole history, none of
              which has been published. */}
          {info.ahead > 0 && (
            <span title={
              info.upstream ? `${info.ahead} ahead of ${info.upstream}`
                : info.base ? `${info.ahead} ahead of the base branch`
                : `${info.ahead} commit${info.ahead === 1 ? "" : "s"} not published anywhere`
            }>{info.ahead}↑</span>
          )}
        </span>
      )}
      <span className="spacer" style={{ flex: 1 }} />
      {/* Pull (fast-forward) only when there is an upstream with commits behind. */}
      {info.upstream && info.behind > 0 && (
        <button className="git-iconbtn" title={`Pull ${info.behind} commit${info.behind === 1 ? "" : "s"} from origin`}
          aria-label="Pull from origin" onClick={() => onAct(() => gitPull(taskId), "Pulled")} disabled={busy}>↓{info.behind}</button>
      )}
      {/* Push: ↥ — an arrow leaving for origin. */}
      {info.upstream && (
        <button className="git-iconbtn" title="Push" aria-label="Push to origin"
          onClick={onPush} disabled={busy}>↥</button>
      )}
      {/* Refresh fetches from origin (so ↓behind reflects reality) then reloads
          the working-tree status. With no remote there's nothing to fetch, so it
          just reloads. */}
      <button className="git-iconbtn"
        title={info.hasRemote ? "Refresh: fetch from origin and reload status" : "Refresh status"}
        aria-label="Refresh"
        disabled={busy}
        onClick={() => (info.hasRemote ? onAct(() => gitFetch(taskId), "Fetched") : onRefresh())}>⟲</button>
      <button className="git-iconbtn" title="More Actions…" aria-label="More actions"
        onClick={(e) => setMenu({ ...menuAt(e), view: "overflow" })}>⋯</button>

      {menu && (
        <Menu x={menu.x} y={menu.y} onClose={() => setMenu(null)}
          items={menu.view === "overflow" ? overflowItems({ x: menu.x, y: menu.y }) : pickerItems(menu.branches)} />
      )}
      {confirmForce && (
        <ConfirmDialog title="Force push" danger confirmLabel="Force Push"
          body={`Force-push ${info.branch} to origin (with lease)? Remote commits not present locally will be overwritten.`}
          onConfirm={() => { setConfirmForce(false); onForcePush(); }}
          onCancel={() => setConfirmForce(false)} />
      )}
      {stashPrompt && (
        <PromptDialog title="Stash changes" body="Stashes all changes, including untracked files."
          placeholder="Stash message" confirmLabel="Stash" initial="WIP"
          onConfirm={(m) => { setStashPrompt(false); onAct(() => gitStashPush(taskId, m, true), "Changes stashed"); }}
          onCancel={() => setStashPrompt(false)} />
      )}
      {branchPrompt && (
        <PromptDialog title="Create branch" placeholder="branch name" confirmLabel="Create & Switch"
          onConfirm={(name) => { setBranchPrompt(false); onAct(() => gitCreateBranch(taskId, name, null, true), `Switched to ${name}`); }}
          onCancel={() => setBranchPrompt(false)} />
      )}
    </div>
  );
}
