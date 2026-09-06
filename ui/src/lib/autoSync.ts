/**
 * What the automatic backlog sync remembers about one project, outside the
 * board component.
 *
 * Every field here started as a ref on `IssuesView`, and `IssuesView` unmounts
 * the moment the user looks at anything else: the board is rendered only on
 * its own tab. So a click on Agents and back silently reset all of it. The
 * schedule announced it had given up after three failures and then started
 * over, error toast included, on every return; and with no record of when the
 * last pass ran, arriving at the board fired a fresh one, which made flipping
 * tabs a way to fetch and push as fast as you could click.
 */
export type AutoSchedule = {
  /**
   * Stop until a manual sync. Seeding needs one deliberate answer, and a
   * remote that is not coming back should not toast on a loop.
   */
  paused: boolean;
  /** Consecutive throws. Three is a remote that needs a person. */
  fails: number;
  /**
   * The last pass came back `pushed: false`. Said once, not every two minutes:
   * the board looks synced while the shared copy is not, so silence would
   * misrepresent it, but a remote that refuses one push refuses every push.
   */
  pushFailed: boolean;
  /**
   * The shared tracker's key the schedule has already said this project does
   * not match, or null. Said once per key, like `pushFailed`: the mismatch is
   * recomputed from the directory on every pass, and until the user changes
   * the key every pass finds the same one, so without this the toast came
   * back every two minutes for as long as the board was open.
   */
  keyMismatchSaid: string | null;
  /**
   * When the last pass started, so returning to the board mid-window waits out
   * the remainder rather than starting the clock over.
   */
  lastPassAt: number;
  /**
   * The `auto` setting this record was built under. Changing the setting is the
   * one thing that earns a clean slate; a remount is not.
   */
  setting: boolean;
};

export interface AutoScheduleStore {
  /**
   * This project's record, created on first ask and reset only when `setting`
   * differs from the one it was built under.
   */
  forProject(projectId: string, setting: boolean): AutoSchedule;
}

/**
 * A store is created rather than exported as a bare map so a test can have its
 * own instead of clearing a global between cases.
 */
export function createAutoSchedules(): AutoScheduleStore {
  const byProject = new Map<string, AutoSchedule>();
  return {
    forProject(projectId, setting) {
      const prev = byProject.get(projectId);
      if (prev && prev.setting === setting) return prev;
      const next: AutoSchedule = {
        paused: false,
        fails: 0,
        pushFailed: false,
        keyMismatchSaid: null,
        lastPassAt: 0,
        setting,
      };
      byProject.set(projectId, next);
      return next;
    },
  };
}

/**
 * Whether a pass's key mismatch is worth a toast, and the record updated to
 * say it was (or was not) said. A manual pass always says it: the user pressed
 * Sync and is owed the reason the board is empty. A scheduled pass says it
 * once per key, and starts over once a pass comes back without one, so a
 * mismatch that goes away and comes back is news again.
 */
export function shouldSayKeyMismatch(
  s: Pick<AutoSchedule, "keyMismatchSaid">,
  theirs: string | null,
  auto: boolean,
): boolean {
  if (theirs === null) {
    s.keyMismatchSaid = null;
    return false;
  }
  const say = !auto || s.keyMismatchSaid !== theirs;
  s.keyMismatchSaid = theirs;
  return say;
}

/** The app's one store, keyed by project: a pause is a fact about one remote. */
export const autoSchedules = createAutoSchedules();
