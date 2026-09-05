import { IssueSyncConflict } from "../api";

/**
 * What a sync decided for one project that the user has not answered yet: the
 * row markers, and the report an automatic pass still owes them.
 *
 * Outside the board for the same reason the schedule in `autoSync.ts` is:
 * `IssuesView` is rendered only on the issues tab, so it unmounts whenever the
 * user looks at anything else, and both of these were component state. The
 * prompt names the issues the merge decided for the user and says the rest are
 * "marked on their rows", but a click on Agents and back threw away the prompt
 * and the markers together, so following that instruction landed on rows that
 * said nothing. In memory and not on disk: this describes what the app merged
 * while it was running, and a restart is a clean slate.
 */
export type ConflictReport = {
  /** Issue key to the lines its row marker shows. */
  markers: Record<string, string[]>;
  /** The dialog an automatic pass still owes the user, or null once dismissed. */
  prompt: IssueSyncConflict[] | null;
};

/** One line per contested field, in the shape the row marker renders. */
export function markerLines(conflicts: IssueSyncConflict[]): Record<string, string[]> {
  const next: Record<string, string[]> = {};
  for (const c of conflicts) (next[c.key] ??= []).push(`${c.field}: ${c.detail}`);
  return next;
}

export interface ConflictStore {
  /** What this project has outstanding. Never stores; safe to call in render. */
  forProject(projectId: string): ConflictReport;
  /**
   * Record what a pass decided, and return the report as it now stands.
   *
   * A pass that decided nothing changes nothing. Clearing on every pass was
   * survivable while each one was a button press, but on the automatic
   * schedule the next clean pass wiped the markers a couple of minutes later,
   * and nothing on screen still recorded what had been quietly overwritten.
   *
   * Only an automatic pass raises the prompt: a manual one reports its
   * conflicts in a toast to the person who just pressed Sync. It clears any
   * outstanding prompt for the same reason it replaces the markers, since a
   * prompt left behind would name conflicts this pass has just superseded.
   */
  record(projectId: string, conflicts: IssueSyncConflict[], opts: { auto: boolean }): ConflictReport;
  /** The user answered the prompt. The markers stay; they are the record. */
  dismiss(projectId: string): ConflictReport;
}

/**
 * A store is created rather than exported as a bare map so a test can have its
 * own instead of clearing a global between cases.
 */
export function createConflictStore(): ConflictStore {
  const byProject = new Map<string, ConflictReport>();
  const forProject = (projectId: string): ConflictReport =>
    byProject.get(projectId) ?? { markers: {}, prompt: null };
  const put = (projectId: string, report: ConflictReport) => {
    byProject.set(projectId, report);
    return report;
  };
  return {
    forProject,
    record(projectId, conflicts, { auto }) {
      if (conflicts.length === 0) return forProject(projectId);
      return put(projectId, { markers: markerLines(conflicts), prompt: auto ? conflicts : null });
    },
    dismiss(projectId) {
      return put(projectId, { ...forProject(projectId), prompt: null });
    },
  };
}

/** The app's one store, keyed by project: another board's merge is not this one's. */
export const conflictReports = createConflictStore();
