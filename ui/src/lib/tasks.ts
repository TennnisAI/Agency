import { Project, TaskHit } from "../api";
import { stripExt } from "./docsIndex";

// Pure model for the Home Tasks section (one-stop Phase 8): checkbox hits from
// the backend scanner, grouped per source note. Checkboxes stay markdown —
// issues are for work, checkboxes are for thoughts; promotion is explicit.

export interface TaskGroup {
  project: Project;
  docsDir: string;
  /** Note path relative to the docs dir. */
  path: string;
  /** Basename sans extension (titles would need corpus bodies we don't load). */
  title: string;
  tasks: TaskHit[];
}

/** Group one project's task hits per note, notes by path, tasks by line. */
export function groupTasks(project: Project, docsDir: string, hits: TaskHit[]): TaskGroup[] {
  const byPath = new Map<string, TaskHit[]>();
  for (const h of hits) {
    const list = byPath.get(h.path) ?? [];
    list.push(h);
    byPath.set(h.path, list);
  }
  return [...byPath.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([path, tasks]) => ({
      project,
      docsDir,
      path,
      title: stripExt(path.split("/").pop() ?? path),
      tasks: [...tasks].sort((x, y) => x.line - y.line),
    }));
}

/** Unchecked tasks across all groups. */
export function openTaskCount(groups: TaskGroup[]): number {
  return groups.reduce((n, g) => n + g.tasks.filter((t) => !t.checked).length, 0);
}

/**
 * Body for an issue promoted from a checkbox: a wikilink back to the source
 * note, so Phase 7 mentions connect the two automatically. The source line is
 * left untouched — checking it off stays the user's call.
 */
export function promoteBody(notePath: string): string {
  return `From [[${stripExt(notePath.split("/").pop() ?? notePath)}]]`;
}

/**
 * A source the user hid from the Tasks section (plan documents full of
 * checkboxes drown real todos). Persisted in localStorage; prefix "" hides
 * the whole project, otherwise an exact note path or a folder prefix.
 */
export interface TaskExclusion {
  projectId: string;
  prefix: string;
}

export function isTaskExcluded(projectId: string, path: string, rules: TaskExclusion[]): boolean {
  return rules.some(
    (r) =>
      r.projectId === projectId &&
      (r.prefix === "" || r.prefix === path || path.startsWith(r.prefix + "/")),
  );
}
