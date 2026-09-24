import { BranchInfo } from "../api";

/**
 * Where the project's own checkout stands against origin, as the status bar
 * shows it (AGE-64 for the push side, AGE-240 for pull). It is shown in the
 * same place whichever run is selected, because the checkout is where merges
 * land: a merge you have not pushed, or a push from elsewhere you have not
 * pulled, is a fact about it and not about the agent on screen.
 */
export type CheckoutStatus =
  /** The branch tracks one on origin: `ahead` to push, `behind` to pull. */
  | { kind: "tracked"; branch: string; upstream: string; ahead: number; behind: number }
  /** There is an origin, but this branch has never been pushed to it. */
  | { kind: "unpublished"; branch: string }
  /** No origin at all: nothing to push to or pull from. */
  | { kind: "local"; branch: string };

export function checkoutStatus(
  info: Pick<BranchInfo, "branch" | "upstream" | "ahead" | "behind" | "hasRemote">,
): CheckoutStatus {
  // `rev-parse --abbrev-ref HEAD` answers the literal "HEAD" when nothing is
  // checked out, which reads like a branch of that name.
  const branch = info.branch === "HEAD" ? "detached HEAD" : info.branch;
  if (info.upstream) {
    return {
      kind: "tracked", branch, upstream: info.upstream, ahead: info.ahead, behind: info.behind,
    };
  }
  // Without an upstream, git's "ahead" is measured from the fork point (or is
  // the whole history) and says nothing about origin, so it is not carried.
  return { kind: info.hasRemote ? "unpublished" : "local", branch };
}

const commits = (n: number) => `${n} commit${n === 1 ? "" : "s"}`;

/** Hover text: what the counts mean, how fresh they are, and where clicking goes. */
export function checkoutStatusTooltip(s: CheckoutStatus): string {
  const where = `Your checkout is on ${s.branch}.`;
  const go = "Opens Source Control for your checkout.";
  if (s.kind === "local") return `${where} It has no remote to push to or pull from. ${go}`;
  if (s.kind === "unpublished") {
    return `${where} It has never been pushed to origin, so Publish is how it gets there. ${go}`;
  }
  const parts: string[] = [];
  if (s.ahead > 0) parts.push(`${commits(s.ahead)} to push`);
  if (s.behind > 0) parts.push(`${commits(s.behind)} to pull`);
  const state = parts.length ? `${parts.join(", ")} against ${s.upstream}` : `In sync with ${s.upstream}`;
  // `behind` is read from the remote-tracking ref, which only moves on a fetch.
  // The app fetches every few minutes and when the window comes back, but a
  // push made elsewhere a moment ago can still read as "in sync".
  return `${where} ${state}, as of the last fetch. ${go}`;
}
