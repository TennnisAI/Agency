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
 * A project folder with no git repository: a workspace that declined git, or a
 * plain folder added as a project. Agents still work there, but in the folder
 * itself, so everything that needs a branch to exist (Source Control, races,
 * loops, GitHub import, the worktree checkbox) hides until a repo is
 * initialized.
 */
export function isGitless(readiness: RepoReadiness | null): boolean {
  return readiness?.state === "notARepo";
}
