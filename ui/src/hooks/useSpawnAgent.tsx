import { useState } from "react";
import { Issue, Project, RepoReadiness, agentInstalled, inspectRepo, startIssueRun } from "../api";
import { useRuns, SpawnOpts } from "../store/runs";
import RepoSetupDialog from "../components/RepoSetupDialog";
import InstallAgentDialog from "../components/InstallAgentDialog";

type Pending = {
  agentId: string;
  readiness: RepoReadiness;
  repoPath: string;
  opts?: SpawnOpts;
  issue?: Issue;
};

/**
 * The pre-flight every agent spawn goes through, plus the dialogs it can raise:
 * a preconfigured agent whose CLI is missing would spawn a session that dies
 * instantly, and a repo that isn't ready has no commit to branch a worktree
 * from. Shared so the Agents tab and the docs/files agents panel offer the same
 * flow instead of drifting apart.
 *
 * Render `dialogs` somewhere in the host tree; `onRepoResolved` fires after the
 * setup dialog fixed the repo, for hosts that cache readiness themselves.
 */
export function useSpawnAgent(project: Project | null, onRepoResolved?: () => void) {
  const { createAgent, refreshRuns, setFocusedRun, setView, setTab } = useRuns();
  const [error, setError] = useState("");
  const [pending, setPending] = useState<Pending | null>(null);
  const [missingAgent, setMissingAgent] = useState<string | null>(null);

  // Dispatch an issue to an agent, then jump to the run — the issue-flavored
  // tail of the same flow createAgent handles for promptless runs.
  async function startIssue(issue: Issue, agentId: string, opts?: SpawnOpts) {
    const run = await startIssueRun(issue.id, agentId, opts?.model ?? null, opts?.base, opts?.mergeTarget);
    await refreshRuns();
    setFocusedRun(run.id);
    setView("focus");
    setTab("agents");
  }

  async function spawn(agentId: string, opts?: SpawnOpts, issue?: Issue) {
    if (!project) return;
    setError("");
    try {
      const installed = await agentInstalled(agentId).catch(() => true);
      if (!installed) {
        setMissingAgent(agentId);
        return;
      }
      const r = await inspectRepo(project.repo_path);
      // The folder is not there at all (AGE-203). Nothing this hook can raise
      // fixes that: the setup dialog only offers `git init` and a first commit,
      // both of which need a folder to run in. Say what is wrong and stop. The
      // project's own screen carries Locate folder and Remove project; this is
      // for the surfaces that spawn from outside it, like the sidebar menu.
      if (r.state === "missing") {
        setError(`${project.repo_path} is missing, so nothing can start there. Open the project to reconnect it or remove it.`);
        return;
      }
      // A folder with no repository has nothing to set up: there is no branch to
      // cut from, so the agent works in the folder as it stands. The add menu
      // already clears the worktree flag here; this restates it so a stale menu
      // (or a caller that never had one) can't ask for a worktree that the
      // backend would only refuse.
      if (r.state === "notARepo") {
        if (issue) await startIssue(issue, agentId, opts);
        // base/mergeTarget are ignored for a run that stays in the checkout;
        // they're spelled only because SpawnOpts asks for them.
        else await createAgent(agentId, { base: "HEAD", mergeTarget: "", worktree: false });
        return;
      }
      // A dirty checkout only blocks cutting a worktree (the new branch would
      // miss the uncommitted work). An agent that stays in the checkout is
      // being started *because* there is work in progress there.
      const dirtyBlocks = opts?.worktree !== false;
      if (r.state === "ready" && !(r.dirty && dirtyBlocks)) {
        if (issue) await startIssue(issue, agentId, opts);
        else await createAgent(agentId, opts);
      } else {
        setPending({ agentId, readiness: r, repoPath: project.repo_path, opts, issue });
      }
    } catch (e) {
      setError(String(e));
    }
  }

  const dialogs = (
    <>
      {pending && (
        <RepoSetupDialog
          readiness={pending.readiness}
          context="spawn"
          repoPath={pending.repoPath}
          onResolved={async () => {
            const { agentId, opts, issue } = pending;
            setPending(null);
            onRepoResolved?.();
            try {
              if (issue) await startIssue(issue, agentId, opts);
              else await createAgent(agentId, opts);
            } catch (e) {
              setError(String(e));
            }
          }}
          onCancel={() => setPending(null)}
        />
      )}
      {missingAgent && project && (
        <InstallAgentDialog
          agent={missingAgent}
          projectId={project.id}
          onInstalling={async (run) => {
            setMissingAgent(null);
            await refreshRuns();
            setFocusedRun(run.id);
            setView("focus");
          }}
          onCancel={() => setMissingAgent(null)}
        />
      )}
    </>
  );

  return { spawn, error, dialogs };
}
