import { useEffect, useRef, useState } from "react";
import { Project, RepoReadiness, inspectRepo } from "../api";

/**
 * Live `inspect_repo` result for a project, shared by every surface that has to
 * know whether agents can spawn here (the Agents tab, the docs/files agents
 * panel). Null while the first inspection is in flight.
 */
export function useRepoReadiness(project: Project | null) {
  const [readiness, setReadiness] = useState<RepoReadiness | null>(null);
  // Request token: a slow inspectRepo from a previous project (or an older
  // refresh) must not land over the current one's readiness.
  const seqRef = useRef(0);
  const repoPath = project?.repo_path ?? null;

  const refresh = () => {
    const seq = ++seqRef.current;
    if (!repoPath) {
      setReadiness(null);
      return;
    }
    inspectRepo(repoPath)
      .then((r) => {
        if (seq === seqRef.current) setReadiness(r);
      })
      .catch(() => {});
  };
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    setReadiness(null);
    refreshRef.current();
  }, [repoPath]);

  return { readiness, refresh };
}

/**
 * A workspace that declined git: agents need a worktree to branch from, so
 * everything git-shaped hides and only terminals remain.
 */
export function isGitlessWorkspace(project: Project | null, readiness: RepoReadiness | null): boolean {
  return project?.kind === "workspace" && readiness?.state === "notARepo";
}
