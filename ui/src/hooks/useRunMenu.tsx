import { useCallback, useEffect, useState } from "react";
import { AgentProfile, RunInfo, listProfiles, renameRun, renameRunBranch, startRunSession } from "../api";
import { agentLabel } from "../agents";
import { useRuns } from "../store/runs";
import { toastError } from "../lib/toast";
import { Removal, removalLabel, removalsFor } from "../lib/runRemoval";
import { attentionEntries } from "../components/AttentionMarker";
import Menu, { MenuEntry } from "../components/git/Menu";
import PromptDialog from "../components/PromptDialog";
import RunRemoveDialog from "../components/RunRemoveDialog";

// Agent profiles change rarely and every run menu offers the same list, so the
// read is shared: a board of twenty tiles each mounting its own menu would
// otherwise fire twenty identical IPC calls at once.
let profileCache: AgentProfile[] = [];
let profilesInFlight: Promise<AgentProfile[]> | null = null;

function loadProfiles(): Promise<AgentProfile[]> {
  if (!profilesInFlight) {
    profilesInFlight = listProfiles()
      .then((ps) => {
        profileCache = ps;
        return ps;
      })
      // A failed read leaves the last good list standing rather than emptying
      // the menu; the next open tries again.
      .catch(() => profileCache)
      .finally(() => {
        profilesInFlight = null;
      });
  }
  return profilesInFlight;
}

/** The enabled agent profiles, for a menu that offers to start one. */
export function useAgentProfiles(): { profiles: AgentProfile[]; reload: () => void } {
  const [profiles, setProfiles] = useState<AgentProfile[]>(profileCache);
  const reload = useCallback(() => {
    loadProfiles().then(setProfiles);
  }, []);
  useEffect(reload, [reload]);
  return { profiles, reload };
}

/**
 * The agent list every "new agent" submenu offers, in the add menu's own words
 * so a right-click and a "+" reach the same place by the same names. "shell" is
 * not an agent profile — it is the reserved name for a login shell, and the
 * caller decides what that means where it is (an extra tab in a workspace, or a
 * new terminal in a project).
 */
export function newAgentItems(profiles: AgentProfile[], start: (agent: string) => void): MenuEntry[] {
  return [
    ...profiles.map((p) => ({ label: agentLabel(p.name), onClick: () => start(p.name) })),
    { kind: "separator" as const },
    { label: "≳ New terminal", onClick: () => start("shell") },
  ];
}

/** What a host renders and calls to give its runs a right-click menu. */
export interface RunMenu {
  /** Hang the menu off a right-click on a run's row, tile or card. */
  openRunMenu: (e: React.MouseEvent, run: RunInfo, onOpen?: (run: RunInfo) => void) => void;
  /** The same entries' actions, for a host that already has a menu of its own. */
  rename: (run: RunInfo) => void;
  renameBranch: (run: RunInfo) => void;
  remove: (run: RunInfo, action: Removal) => void;
  /**
   * The run whose menu is open, for the host to mark its row with: the menu's
   * overlay swallows `:hover`, so without it the row the menu acts on is the
   * one row that stops looking picked (same fix as `.git-row.ctx`).
   */
  menuRunId: string | null;
  /** Render inside the host: the open menu, plus whatever dialog it raised. */
  runMenu: React.ReactNode;
}

/**
 * The right-click menu a run carries wherever it is listed — the grid tile, the
 * focus rail, the row under its project in the sidebar. One place builds it, so
 * the four surfaces cannot end up offering three different vocabularies for the
 * same four actions (AGE-179).
 *
 * The dialogs the entries raise (rename, archive, delete) are part of `runMenu`
 * rather than the host's own state, which is why the focus view's header menu
 * calls the actions here instead of keeping a second copy of them.
 *
 * `onOpen` overrides what "Open" does — the sidebar tree has to select the run's
 * project first, since the run may belong to one that isn't open.
 */
export function useRunMenu({
  onOpen,
  onChanged,
}: {
  onOpen?: (run: RunInfo) => void;
  /** The host's list needs re-reading after a rename or a removal. */
  onChanged?: () => void;
} = {}): RunMenu {
  const { setFocusedRun, setView, setTab, requestAgentView, setPendingSession, refreshRuns } = useRuns();
  const { profiles, reload } = useAgentProfiles();
  const [menu, setMenu] = useState<{ x: number; y: number; run: RunInfo; onOpen?: (run: RunInfo) => void } | null>(null);
  const [pending, setPending] = useState<{ run: RunInfo; action: Removal } | null>(null);
  const [renaming, setRenaming] = useState<RunInfo | null>(null);
  const [renamingBranch, setRenamingBranch] = useState<RunInfo | null>(null);

  // The store's own list, plus whatever separate list the host is showing (the
  // sidebar tree holds runs from every project, not just the selected one).
  const changed = () => {
    refreshRuns();
    onChanged?.();
  };

  const openRun = (run: RunInfo, override?: (run: RunInfo) => void) => {
    const custom = override ?? onOpen;
    if (custom) {
      custom(run);
      return;
    }
    setTab("agents");
    setFocusedRun(run.id);
    setView("focus");
    requestAgentView(run.id);
  };

  // An extra agent (or shell) sharing this run's worktree — the same thing the
  // "+" in the session strip does, reachable without opening the run first.
  const newSession = async (run: RunInfo, agent: string, override?: (run: RunInfo) => void) => {
    let session;
    try {
      session = await startRunSession(run.id, agent);
    } catch (e) {
      toastError(e, agent === "shell" ? "Couldn't open terminal" : "Couldn't open agent tab");
      return;
    }
    // Open first, hand the tab over second: opening a run in a project that
    // isn't the current one goes through setSelectedProject, which clears any
    // pending session, and the later write is the one that survives the batch.
    openRun(run, override);
    setPendingSession(session.id);
  };

  const entriesFor = (m: NonNullable<typeof menu>): MenuEntry[] => {
    const { run } = m;
    const isTerminal = run.kind === "terminal";
    const entries: MenuEntry[] = [
      { label: isTerminal ? "Open terminal" : "Open agent", onClick: () => openRun(run, m.onOpen) },
    ];
    // Extra tabs live in the focus view's session strip, which a terminal run
    // doesn't have: its whole pane is the one shell.
    if (!isTerminal) {
      entries.push({
        kind: "submenu",
        label: "New agent in this workspace",
        items: newAgentItems(profiles, (agent) => void newSession(run, agent, m.onOpen)),
      });
    }
    entries.push(
      { kind: "separator" },
      { label: isTerminal ? "Rename terminal…" : "Rename agent…", onClick: () => setRenaming(run) },
    );
    // A run working in the project's own checkout is on the user's branch, not
    // one Agency cut for it, so there is nothing here to rename.
    if (run.worktree) {
      entries.push({ label: "Rename branch…", onClick: () => setRenamingBranch(run) });
    }
    entries.push({ kind: "separator" }, ...attentionEntries(run, changed), { kind: "separator" });
    for (const action of removalsFor(run)) {
      entries.push({
        label: removalLabel(run, action),
        danger: action === "delete",
        onClick: () => setPending({ run, action }),
      });
    }
    return entries;
  };

  const openRunMenu = (e: React.MouseEvent, run: RunInfo, onOpenOverride?: (run: RunInfo) => void) => {
    e.preventDefault();
    e.stopPropagation();
    // A profile added moments ago should be in the list this open builds.
    reload();
    setMenu({ x: e.clientX, y: e.clientY, run, onOpen: onOpenOverride });
  };

  // Hosts that mount this inside a clickable row (a tile opens the run when
  // clicked) would otherwise see every menu click as a click on the row.
  // `display: contents` keeps the wrapper out of the host's layout.
  const anyOpen = menu || pending || renaming || renamingBranch;
  const runMenu = anyOpen ? (
    <span className="run-menu-host" onClick={(e) => e.stopPropagation()}>
      {menu && <Menu x={menu.x} y={menu.y} items={entriesFor(menu)} onClose={() => setMenu(null)} />}
      {pending && (
        <RunRemoveDialog
          run={pending.run}
          action={pending.action}
          onClose={() => setPending(null)}
          onRemoved={() => onChanged?.()}
        />
      )}
      {renamingBranch && (
        <PromptDialog
          title="Rename branch"
          body="The merge commit carries this name into the base branch's history for good, and a merged PR's branch can't be renamed after the fact. The agent, its workspace and its work stay where they are."
          placeholder="agent/some-name"
          initial={renamingBranch.branch}
          confirmLabel="Rename"
          onConfirm={(v) => {
            const id = renamingBranch.id;
            setRenamingBranch(null);
            renameRunBranch(id, v).then(changed).catch((e) => toastError(e, "Rename failed"));
          }}
          onCancel={() => setRenamingBranch(null)}
        />
      )}
      {renaming && (
        <PromptDialog
          title={renaming.kind === "terminal" ? "Rename terminal" : "Rename agent"}
          placeholder="New name"
          initial={renaming.title ?? ""}
          confirmLabel="Rename"
          onConfirm={(v) => {
            const id = renaming.id;
            setRenaming(null);
            renameRun(id, v).then(changed).catch((e) => toastError(e, "Rename failed"));
          }}
          onCancel={() => setRenaming(null)}
        />
      )}
    </span>
  ) : null;

  return {
    openRunMenu,
    menuRunId: menu?.run.id ?? null,
    rename: setRenaming,
    renameBranch: setRenamingBranch,
    remove: (run, action) => setPending({ run, action }),
    runMenu,
  };
}
