import { useEffect, useRef, useState } from "react";
import { Project, folderMissing } from "../api";

// How often a project's folder is checked for. One `stat` per project, so the
// cost is nothing next to the boards' own 1.5-2.5s polls; this is slower only
// because a folder rarely moves twice in a row.
const FOLDER_POLL_MS = 5_000;


/**
 * Whether a project's source folder has gone from disk (AGE-203): moved,
 * renamed, deleted, or on a volume that is no longer mounted.
 *
 * Polled rather than read once, because this is the one property of a project
 * that changes without the app being involved at all: the user drags the folder
 * in Finder, or ejects the disk it is on, while Agency is in the background.
 * Every tab then polls its own data on its own timer and fails in its own way,
 * which is the mix of errors and loading screens the issue is about; this is
 * the single question that replaces all of them with one answer.
 *
 * `null` until the first probe lands, so nothing flashes a missing-folder
 * screen at a project that is perfectly fine.
 */
export function useFolderMissing(project: Project | null): {
  missing: boolean | null;
  refresh: () => void;
} {
  const [missing, setMissing] = useState<boolean | null>(null);
  const repoPath = project?.repo_path ?? null;
  // Request token: a slow probe from the previously selected project must not
  // land over the current one's answer.
  const seqRef = useRef(0);

  const refresh = () => {
    const seq = ++seqRef.current;
    if (!repoPath) {
      setMissing(null);
      return;
    }
    folderMissing(repoPath)
      .then((m) => {
        if (seq === seqRef.current) setMissing(m);
      })
      // No answer is no claim: leave the last one standing rather than
      // accusing a working folder of being gone because one IPC call failed.
      .catch(() => {});
  };
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    setMissing(null);
    refreshRef.current();
    const t = window.setInterval(() => refreshRef.current(), FOLDER_POLL_MS);
    // A folder is usually moved or a disk ejected while Agency is in the
    // background, so coming back to the window is the moment the answer is
    // most likely to have changed, and the moment the user is looking.
    const onFocus = () => refreshRef.current();
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  }, [repoPath]);

  return { missing, refresh };
}

/**
 * Which of `projects` have lost their folder, by project id. For the sidebar,
 * which lists them all and is where a missing folder should be visible without
 * clicking into the project first.
 *
 * One probe per folder per sweep, keyed by folder rather than project id: the
 * question is about the folder, and two projects on one folder share the answer.
 */
export function useMissingFolders(projects: Project[]): Set<string> {
  const [missingBy, setMissingBy] = useState<Record<string, boolean>>({});
  // A stable effect dependency: the array itself is rebuilt by every refresh,
  // identical contents and all.
  const key = projects.map((p) => p.repo_path).join("\n");

  useEffect(() => {
    let live = true;
    const paths = key ? [...new Set(key.split("\n"))] : [];
    const sweep = async () => {
      const answers = await Promise.all(
        paths.map((p) => folderMissing(p).then((m) => [p, m] as const).catch(() => null)),
      );
      if (!live) return;
      const found = answers.filter((a): a is readonly [string, boolean] => a !== null);
      if (found.length === 0) return;
      // Merged, not replaced: a probe that failed is absent from `found`, and
      // replacing the whole map with it dropped every other folder's answer
      // too, so one hung network mount cleared the ⚠︎ from every other
      // project's row until a later sweep happened to succeed. No answer is no
      // claim, here as in the single-project hook above.
      //
      // Only when an answer actually changed: the sidebar re-renders every
      // project row and its agents, and a sweep that says the same thing as
      // the last one has nothing to show for it.
      setMissingBy((prev) =>
        found.every(([p, m]) => prev[p] === m) ? prev : { ...prev, ...Object.fromEntries(found) },
      );
    };
    void sweep();
    const t = window.setInterval(() => void sweep(), FOLDER_POLL_MS);
    const onFocus = () => void sweep();
    window.addEventListener("focus", onFocus);
    return () => {
      live = false;
      window.clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  }, [key]);

  return new Set(projects.filter((p) => missingBy[p.repo_path]).map((p) => p.id));
}
