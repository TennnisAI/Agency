// gh's refusals, as they reach us, put in terms of what to do next. The raw
// text is kept when nothing here recognises it: it is gh's own explanation,
// and a protected-branch rule or a required review says more than we could.
export function prMergeError(e: unknown, base: string): string {
  const raw = String(e);
  // GitHub refuses a PR that collides with its base: "Pull request is not
  // mergeable: the merge commit cannot be cleanly created." Matched on the
  // reason, not on "not mergeable": gh opens every refusal that way, a
  // protected branch's "the base branch policy prohibits the merge" and a
  // stale head's "the head branch is not up to date with the base branch"
  // included, and both were being told to fix a conflict they did not have.
  if (/cleanly created|merge conflict/i.test(raw)) {
    return `GitHub can't merge this pull request cleanly: it conflicts with ${base}. Merge ${base} into the branch, push, and try again.`;
  }
  // `--match-head-commit` tripped: the PR's head is not the branch's local
  // tip, so what would land is not what is in the worktree. Not matched on
  // the flag's own name, which is in every failed pinned merge's error
  // because the backend's error quotes gh's arguments.
  if (/head branch was modified|expected head sha/i.test(raw)) {
    return "The branch on GitHub doesn't match this agent's branch. Pull or push it in Source Control so both sides agree, then try again.";
  }
  return raw;
}
