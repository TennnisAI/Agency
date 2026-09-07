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
 *
 * A folder that has gone from disk ("missing") is deliberately not one of
 * these, even though it has no repository in it either: this answer sends an
 * agent into the folder as it stands, and there is no folder. That is
 * `useFolderMissing`, which polls a single `stat` rather than a whole
 * `inspect_repo`, and whose panel replaces the tabs outright (AGE-203).
 */
export function isGitless(readiness: RepoReadiness | null): boolean {
  return readiness?.state === "notARepo";
}

// How often a folder still believed to have no repository is asked again.
// Much slower than the boards that use it poll (2.5s), because this is a
// self-heal for `git init` happening elsewhere, not a live reading.
const GITLESS_RECHECK_MS = 30_000;

/**
 * The paths a sweep should inspect: the ones with no answer yet, plus the ones
 * last seen gitless. A folder can gain a repository under us (`git init` in a
 * terminal, or the setup dialog), and that is the direction worth re-asking
 * about, since it puts the branch-shaped menu entries back. A repository does
 * not stop being one.
 */
export function pathsToProbe(paths: string[], known: Record<string, boolean>): string[] {
  return [...new Set(paths)].filter((p) => known[p] !== false);
}

/**
 * Which of `projects` sit in a folder with no git repository, by project id.
 * For surfaces listing many projects at once — the cross-project home board,
 * whose issue rows must not offer "Race agents…" or "Loop agent…" for a folder
 * with no branch to cut a worktree from.
 *
 * One `inspect_repo` per folder the first time it is seen, not one per project
 * per poll: the board re-reads every project's issues and runs every 2.5s, and
 * git subprocesses on that cadence would be pure waste for an answer that
 * changes at most once in a folder's life. Single-project surfaces want
 * `useRepoReadiness` instead, which reads the full readiness live.
 */
export function useGitlessProjects(projects: Project[]): Set<string> {
  // Keyed by folder, not project id: the question is about the folder, and two
  // projects on one folder share the answer.
  const [gitlessBy, setGitlessBy] = useState<Record<string, boolean>>({});
  const known = useRef(gitlessBy);
  known.current = gitlessBy;
  // One probe per folder at a time: `inspect_repo` runs `git status` on a whole
  // tree, so a slow one must not have the next sweep pile a second on top of it.
  const inFlight = useRef(new Set<string>());
  // A stable effect dependency: the array itself is rebuilt by every poll tick,
  // identical contents and all.
  const key = projects.map((p) => p.repo_path).join("\n");

  useEffect(() => {
    let live = true;
    const sweep = () => {
      for (const path of pathsToProbe(key ? key.split("\n") : [], known.current)) {
        if (inFlight.current.has(path)) continue;
        inFlight.current.add(path);
        inspectRepo(path)
          .then((r) => {
            if (live) setGitlessBy((m) => ({ ...m, [path]: isGitless(r) }));
          })
          // No answer means no claim: the menu keeps its branch entries and the
          // backend's refusal is what the user sees, exactly as before.
          .catch(() => {})
          .finally(() => inFlight.current.delete(path));
      }
    };
    sweep();
    const t = window.setInterval(sweep, GITLESS_RECHECK_MS);
    return () => {
      live = false;
      window.clearInterval(t);
    };
  }, [key]);

  return new Set(projects.filter((p) => gitlessBy[p.repo_path]).map((p) => p.id));
}
