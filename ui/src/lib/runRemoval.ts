import { CleanupPlan, RunCleanup, RunInfo } from "../api";

/**
 * The two ways a run leaves the board: archived (put away, its record kept) or
 * deleted (gone, record and all).
 *
 * What each one *removes* is not decided here — it is `cleanup.rs`'s answer,
 * arriving as a {@link CleanupPlan}, so the words below and the git commands
 * the backend runs cannot drift apart. This module only turns that plan into
 * sentences.
 */
export type Removal = "archive" | "delete";

export type RemovalCopy = {
  title: string;
  /** The sentence above the lists. */
  lead: string;
  /** What this teardown removes, one clause each. Never empty in practice. */
  goes: string[];
  /** What survives it. */
  stays: string[];
  /**
   * The line that earns a red button: work that exists nowhere but the branch
   * about to be deleted. Null whenever nothing is at risk, which after a merge
   * is always — a delete that cannot lose anything must not be dressed as one
   * that can, or the warning stops being read on the day it matters.
   */
  warning: string | null;
  confirmLabel: string;
  danger: boolean;
  /** Toast title when the call fails. */
  failTitle: string;
};

/** Only the run fields the wording depends on. */
export type RemovalRun = Pick<RunInfo, "kind" | "agent" | "branch" | "worktree">;

/** Menu label for a removal, in the vocabulary of the run it acts on. */
export function removalLabel(run: Pick<RunInfo, "kind">, action: Removal): string {
  if (action === "archive") return "Archive agent";
  return run.kind === "terminal" ? "Close terminal" : "Delete agent";
}

/** Which removals a run offers: a terminal can only be closed. */
export function removalsFor(run: Pick<RunInfo, "kind">): Removal[] {
  return run.kind === "terminal" ? ["delete"] : ["archive", "delete"];
}

// A run can be removed from a tile, from the focus header, from the agents rail
// or from the merge window, and it should read the same in all four — so the
// wording lives here, once, next to the distinctions it has to make: a terminal
// owns nothing but a shell, and a run without a worktree works in the user's own
// checkout, where nothing may be committed or thrown away on its behalf.
//
// `cleanup` is null while the plan is still being read, and stays null if the
// probe failed. The copy then says only what is true of every run, and claims
// nothing about the branch — the backend still decides correctly either way.
export function removalCopy(
  run: RemovalRun,
  action: Removal,
  cleanup: RunCleanup | null,
): RemovalCopy {
  if (run.kind === "terminal") {
    return {
      title: "Close terminal?",
      lead: "Stop the shell and remove this terminal session.",
      goes: [],
      stays: [],
      warning: null,
      confirmLabel: "Close",
      danger: false,
      failTitle: "Close failed",
    };
  }
  const plan = cleanup?.[action] ?? null;
  const branch = cleanup?.branch || run.branch;
  const base = cleanup?.base || "the base branch";

  // A run in the project's own checkout has no worktree and no branch of ours.
  // Its changes are the user's, sitting uncommitted where they left them, and
  // neither verb may touch them.
  if (!run.worktree) {
    return action === "archive"
      ? {
          title: "Archive agent?",
          lead: `Stop "${run.agent}" and file the run away.`,
          goes: ["Its session."],
          stays: [
            "Your checkout and everything in it. Nothing is committed or removed.",
            "A record of the run, under Archived, where you can restore it.",
          ],
          warning: null,
          confirmLabel: "Archive",
          danger: false,
          failTitle: "Archive failed",
        }
      : {
          title: "Delete agent?",
          lead: `Stop "${run.agent}" and remove the run.`,
          goes: ["Its session and its record."],
          stays: ["Your checkout and its changes, exactly as they are."],
          warning: null,
          confirmLabel: "Delete",
          danger: false,
          failTitle: "Delete failed",
        };
  }

  // Uncommitted work is only worth a sentence when there is some: on a clean
  // worktree the promise to commit it first is noise that makes the list
  // longer and the decision harder.
  const dirty = cleanup?.facts.dirty ?? false;
  const goes: string[] = [
    action === "archive"
      ? dirty
        ? `Its worktree. What is uncommitted there is committed to ${branch} first.`
        : "Its worktree."
      : dirty
        ? "Its worktree, and the uncommitted changes in it."
        : "Its worktree.",
  ];
  const stays: string[] = [];

  if (plan?.deletesBranch) {
    goes.push(`The ${branch} branch${safeClause(plan, base)}.`);
  } else if (plan?.keepsBranch) {
    stays.push(`The ${branch} branch${heldClause(cleanup?.facts.commitsAhead ?? 0)}.`);
  }
  // With no plan read, nothing is said about the branch at all. The dialog
  // shows that it is still checking; a guess that reads as a promise is worse
  // than a pause.

  // The conversation, but only for the agents Agency can actually find one
  // for: `managesTranscript` is the same test the rescue and the removal make
  // before they touch a file. For every other agent the history stays in the
  // agent's own store, resumable from the agent, and saying so is the
  // difference between a list the user believes and one they have disproved.
  const transcript = cleanup?.managesTranscript ?? false;
  if (action === "archive") {
    stays.push(
      transcript
        ? "A record of what this agent did, and its conversation, under Archived."
        : "A record of what this agent did, under Archived.",
    );
    if (plan?.restorable) stays.push("The agent itself, restorable from that record.");
  } else {
    goes.push("The run and its record.");
    if (transcript) goes.push("Its conversation, from the agent's own session store.");
    else stays.push("This agent's own session history, which Agency does not keep or remove.");
  }

  const atRisk = plan?.commitsAtRisk ?? 0;
  const losesUncommitted = plan?.losesUncommitted ?? false;
  return {
    title: action === "archive" ? "Archive agent?" : "Delete agent?",
    lead:
      action === "archive"
        ? `Stop "${run.agent}" and put this run away.`
        : `Stop "${run.agent}" and remove this run.`,
    goes,
    stays,
    warning: warningFor(atRisk, losesUncommitted, branch, base),
    confirmLabel: action === "archive" ? "Archive" : "Delete",
    danger: atRisk > 0 || losesUncommitted,
    failTitle: action === "archive" ? "Archive failed" : "Delete failed",
  };
}

/**
 * The last step of a merge, where three buttons are the whole decision:
 * archive the agent, delete it, or keep it.
 *
 * That step used to show the *archive* teardown's Removes/Keeps lists under a
 * heading, "Tidy up", that matched none of the three buttons: it explained one
 * option in detail and left the other two unsaid, which is what made the step
 * read as a puzzle (AGE-164). The same facts are here as one line per button,
 * in the buttons' own verbs.
 *
 * The distinction each line has to carry is what happens to the *transcript*,
 * because that is the only thing the two teardowns treat differently now that
 * both can be undone. And it is conditional: `managesTranscript` is true only
 * for a worktree run by an agent whose format Agency reads, which is two agents
 * out of the list. For everyone else neither verb goes near the conversation
 * and the agent's own resume still finds it, which is worth saying out loud —
 * users delete agents and then notice their history is still there, and copy
 * that claims otherwise is copy they stop believing.
 */
export type MergeTidyCopy = {
  /** The buttons, in the order they appear, each with what it leaves behind. */
  choices: { verb: string; text: string }[];
  /**
   * The mechanics under the three lines: the branch's real fate, that the merge
   * itself is untouched, and that archiving is reversible. Null while the plan
   * is unread, where the window says it is still checking rather than guessing.
   */
  detail: string | null;
  /**
   * Work that only one of the two verbs saves. Shown even when the rest of the
   * explanation is hushed, because it is the one line that decides which button
   * to press.
   */
  caveat: string | null;
};

export function mergeTidyCopy(cleanup: RunCleanup | null): MergeTidyCopy {
  const branch = cleanup?.branch || "";
  const base = cleanup?.base || "the base branch";
  // With no plan read, the branch is left out of the sentence entirely: the
  // worktree always goes and the transcript is always kept, so those two are
  // safe to say about any run, and the branch is not.
  const takesBranch = !!branch && !!cleanup?.archive.deletesBranch;
  // Default-deny: with no plan read we do not yet know whether this agent's
  // conversation is one Agency moves, so neither line claims anything about it.
  const transcript = !!cleanup?.managesTranscript;
  const kept = transcript ? "the transcript and a record" : "a record of what it did";
  const choices = [
    {
      verb: "Archive",
      text: takesBranch
        ? `removes the worktree and the ${branch} branch, and keeps ${kept} under Archived`
        : `removes the worktree, and keeps ${kept} under Archived`,
    },
    {
      verb: "Delete",
      text: transcript ? "removes all of it, the transcript included" : "removes all of it",
    },
    { verb: "Keep", text: "leaves the agent as it is" },
  ];
  if (!cleanup) return { choices, detail: null, caveat: null };

  const { archive, delete: remove } = cleanup;
  const parts: string[] = [];
  // They part company over the branch whenever it is still the only copy of
  // something — uncommitted work that archiving commits to it, say — and a
  // shared clause that swallows the branch when only one verb takes it is
  // exactly the drift this module exists to prevent.
  if (branch && archive.keepsBranch && remove.deletesBranch) {
    parts.push(`Only deleting takes the ${branch} branch with it.`);
  } else if (takesBranch) {
    const why = safeReason(archive, base);
    if (why) parts.push(`The ${branch} branch goes either way, since it ${why}.`);
  }
  parts.push("Neither touches what you just merged.");
  // The one fact that makes Archive the low-stakes button: it is not a
  // one-way door. Restore puts the worktree back — cutting the branch again
  // from the base when the archive let it go — and reinstates the conversation
  // in it, so the agent picks up where it stopped.
  if (archive.restorable) {
    parts.push("Archiving can be undone: Restore brings the worktree and the conversation back.");
  }
  // Said only where it is true. Agency moves the conversation for the agents
  // whose transcript layout it knows; every other agent keeps its own session
  // history wherever it puts it, and neither button here reaches that.
  if (!transcript) {
    parts.push("This agent keeps its own session history, which neither one touches.");
  }

  const caveats: string[] = [];
  if (remove.losesUncommitted) {
    caveats.push(
      `Archiving commits what is uncommitted in the worktree to ${branch}; deleting discards it.`,
    );
  }
  const risk = warningFor(remove.commitsAtRisk, false, branch, base);
  if (risk) caveats.push(risk);

  return {
    choices,
    detail: parts.join(" "),
    caveat: caveats.length ? caveats.join(" ") : null,
  };
}

/**
 * The red-button line. Two different things can be lost and they are lost for
 * different reasons, so they are said separately rather than folded into one
 * vague "this cannot be undone".
 */
function warningFor(
  atRisk: number,
  losesUncommitted: boolean,
  branch: string,
  base: string,
): string | null {
  const parts: string[] = [];
  if (atRisk > 0) {
    parts.push(
      `${atRisk} commit${atRisk === 1 ? "" : "s"} on ${branch} ${
        atRisk === 1 ? "is" : "are"
      } not on ${base} and not on any remote. Deleting the branch loses ${
        atRisk === 1 ? "it" : "them"
      }.`,
    );
  }
  if (losesUncommitted) {
    parts.push("Uncommitted changes in its worktree go with it, on any branch.");
  }
  return parts.length ? parts.join(" ") : null;
}

/** Why letting this branch go costs nothing, as a predicate. Null if it does. */
function safeReason(plan: CleanupPlan, base: string): string | null {
  switch (plan.safeBecause) {
    case "merged":
      return `is already on ${base}`;
    case "pushed":
      return "is already on a remote";
    case "empty":
      return "has no commits of its own";
    default:
      return null;
  }
}

/** Why deleting this branch costs nothing, as a trailing relative clause. */
function safeClause(plan: CleanupPlan, base: string): string {
  switch (plan.safeBecause) {
    case "merged":
      return `, which is already on ${base}`;
    case "pushed":
      return ", which is already on a remote";
    case "empty":
      return ", which has no commits of its own";
    default:
      return "";
  }
}

/** What a kept branch is holding, and therefore why it is being kept. */
function heldClause(commits: number): string {
  if (!commits) return ", the only copy of this work";
  return `, carrying ${commits} commit${commits === 1 ? "" : "s"} that ${
    commits === 1 ? "exists" : "exist"
  } nowhere else`;
}
