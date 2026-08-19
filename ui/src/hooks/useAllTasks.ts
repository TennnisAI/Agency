import { useCallback, useEffect, useRef, useState } from "react";
import {
  FileRoot,
  Project,
  TaskHit,
  detectDocsDir,
  docsCorpusStats,
  listProjects,
  scanTasks,
  toggleTask,
} from "../api";
import { TaskGroup, groupTasks } from "../lib/tasks";

const POLL_MS = 5000;

interface ProjectTasks {
  project: Project;
  docsDir: string;
  hits: TaskHit[];
}

/**
 * Checkbox tasks across the workspace and every project (one-stop Phase 8),
 * fan-out polled like useCrossRefs. A per-project corpus stats signature gates
 * the actual scan, so an idle Home costs one readdir per project per tick and
 * zero body reads. Per-project failures drop that project for the tick;
 * `groups` is null until the first pass lands.
 */
export function useAllTasks(active: boolean) {
  const [groups, setGroups] = useState<TaskGroup[] | null>(null);
  const token = useRef(0);
  const sigs = useRef(new Map<string, string>());
  const cache = useRef(new Map<string, ProjectTasks>());

  const refresh = useCallback(async (force = false) => {
    const t = ++token.current;
    try {
      const projects = await listProjects();
      const per = await Promise.all(
        projects.map(async (project): Promise<ProjectTasks | null> => {
          try {
            const dir = await detectDocsDir(project.id);
            if (dir === null) return null;
            const root: FileRoot = { kind: "project", id: project.id };
            const scan = await docsCorpusStats(root, dir);
            // Tasks live in files, so an empty folder appearing is not a reason
            // to rescan.
            const sig = scan.files
              .map((s) => `${s.path}:${s.mtimeMs}:${s.size}`)
              .sort()
              .join("|");
            const prev = cache.current.get(project.id);
            if (!force && prev && prev.docsDir === dir && sigs.current.get(project.id) === sig) {
              return { ...prev, project };
            }
            const hits = await scanTasks(root, dir);
            const entry = { project, docsDir: dir, hits };
            cache.current.set(project.id, entry);
            sigs.current.set(project.id, sig);
            return entry;
          } catch {
            return null;
          }
        }),
      );
      if (token.current !== t) return;
      setGroups(
        per
          .filter((p): p is ProjectTasks => p !== null)
          .flatMap((p) => groupTasks(p.project, p.docsDir, p.hits)),
      );
    } catch {
      /* transient IPC errors: keep the last good data */
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    void refresh();
    const t = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(t);
  }, [active, refresh]);

  /**
   * Flip one checkbox in its source file. Resolves false when the line moved
   * under us (external edit) — the refresh that follows re-syncs either way.
   */
  const toggle = useCallback(
    async (group: TaskGroup, task: TaskHit, checked: boolean): Promise<boolean> => {
      const root: FileRoot = { kind: "project", id: group.project.id };
      const ok = await toggleTask(root, group.docsDir, task.path, task.line, checked);
      await refresh(true);
      return ok;
    },
    [refresh],
  );

  return { groups, toggle, refresh };
}
